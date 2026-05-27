# BI-0 MoriPack Artifact Handoff Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把已存在的 character pack 匯入流程，正式建模成 Body Interface 的 **artifact handoff envelope**，並讓 handoff「可見、可取消」——使 MoriPack 成為 Body Interface 第一個被驗證的 artifact-first 樣板。

**Architecture:** 新增一個平台無關的 semantic 型別 `MoriArtifact`（對應 `docs/mori-body-interface.md` §Semantic schema 的 `MoriArtifactMetadata`）放在 `mori-core`，加一個純函式 `classify_artifact(path)` 把本機檔案分類成 artifact。`mori-tauri` 加一個 `inspect_artifact` command 讓前端在匯入**前**先看到「這是什麼 / 能做什麼」，使用者確認後才走既有的 `character_pack_import_zip`。**不建** generic artifact dispatcher / registry（只有一種 kind，那是 BI-1+ 的事，YAGNI）。

**Tech Stack:** Rust（mori-core / mori-tauri，serde + uuid + tauri command）、React/TS（ConfigTab `CharacterPicker`）。

**Scope guardrails（來自 backlog 決定 D1 + §1 範圍紀律）:**
- `MoriArtifact` 欄位**只放** doc contract 需要的：`artifact_id / kind / path / visibility / mime / suggested_actions`。
- 不做 ROS2/Zenoh binding、不做 manifest reader、不做 permission broker（那些是 BI-1 / BI-2）。
- 既有的 import / validate / activate / reload **不重寫**，只在它前面接上 artifact inspection + 可見 handoff。

**Spec sources:** `docs/moripack-integration.md`（Artifact Contract L76-112、Desktop Integration Flow L114-152）、`docs/body-interface-backlog.md`（BI-0 row + 完成判準）、`docs/mori-body-interface.md`（§Semantic schema L342-354、§Artifact Handoff L467-481「所有 handoff 都要可見、可取消」）。

---

## File Structure

| 檔案 | 責任 | 動作 |
|---|---|---|
| `crates/mori-core/src/body/mod.rs` | Body Interface semantic 型別的 module root（BI-0 只放 artifact）| Create |
| `crates/mori-core/src/body/artifact.rs` | `MoriArtifact` / `Visibility` / `SuggestedAction` / `classify_artifact` + 測試 | Create |
| `crates/mori-core/src/lib.rs` | 宣告 `pub mod body;` | Modify(`:14` 後插一行）|
| `crates/mori-tauri/src/main.rs` | `inspect_artifact` command + 註冊 | Modify(command 加在 character 區附近、`invoke_handler!` 加一行）|
| `src/tabs/ConfigTab.tsx` | `CharacterPicker`：pick → inspect → 可見/可取消 handoff → confirm import | Modify(`MoriArtifact` interface + 改 `onImport` 拆兩段 + handoff panel）|
| `docs/moripack-integration.md` | §下一步 標 1-3 done + 註明 artifact envelope 已落地 | Modify |
| `docs/body-interface-backlog.md` | BI-0 row → in-progress/done + 附 manual e2e | Modify |

---

## Task 1: `MoriArtifact` semantic 型別

**Files:**
- Create: `crates/mori-core/src/body/artifact.rs`
- Create: `crates/mori-core/src/body/mod.rs`
- Modify: `crates/mori-core/src/lib.rs:14`(在 `pub mod annuli;` 後插 `pub mod body;`，維持大致字母序）

- [ ] **Step 1: 建 `body/mod.rs`**

```rust
//! Body Interface — Mori universe 各身體部件接入 Mori 的 semantic 契約。
//! 見 `docs/mori-body-interface.md`。BI-0 只放 artifact;BI-1+ 再加
//! manifest / event / permission / cue 等型別。

pub mod artifact;

pub use artifact::{
    classify_artifact, MoriArtifact, SuggestedAction, Visibility, KIND_CHARACTER_PACK,
};
```

- [ ] **Step 2: 在 `body/artifact.rs` 寫 failing test（型別還不存在）**

```rust
//! Body Interface 的 semantic artifact envelope。
//!
//! 對應 `docs/mori-body-interface.md` §Semantic schema 的 `MoriArtifactMetadata`
//! 與 `docs/moripack-integration.md` 的 Artifact Contract。raw 內容留在來源,
//! 這個 envelope 只描述「它是什麼、在哪、能對它做什麼」。

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_serializes_to_doc_contract_shape() {
        let a = MoriArtifact {
            artifact_id: "character_pack_001".into(),
            kind: KIND_CHARACTER_PACK.into(),
            path: "/tmp/mori.moripack.zip".into(),
            visibility: Visibility::Local,
            mime: "application/zip".into(),
            suggested_actions: vec![
                SuggestedAction::Validate,
                SuggestedAction::Import,
                SuggestedAction::Activate,
            ],
        };
        let v: serde_json::Value = serde_json::to_value(&a).unwrap();
        assert_eq!(v["kind"], "mori.character-pack");
        assert_eq!(v["visibility"], "local");
        assert_eq!(
            v["suggested_actions"],
            serde_json::json!(["validate", "import", "activate"])
        );
        let back: MoriArtifact = serde_json::from_value(v).unwrap();
        assert_eq!(back, a);
    }

    #[test]
    fn new_generates_prefixed_unique_id() {
        let a = MoriArtifact::new(
            KIND_CHARACTER_PACK,
            "/x.zip",
            Visibility::Local,
            "application/zip",
            vec![],
        );
        let b = MoriArtifact::new(
            KIND_CHARACTER_PACK,
            "/y.zip",
            Visibility::Local,
            "application/zip",
            vec![],
        );
        assert!(a.artifact_id.starts_with("artifact_"));
        assert_ne!(a.artifact_id, b.artifact_id);
    }
}
```

- [ ] **Step 3: 跑測試確認 fail（編譯不過）**

Run: `cargo test -p mori-core --lib body::artifact`
Expected: 編譯失敗 —「cannot find type `MoriArtifact`」之類。

- [ ] **Step 4: 在 test mod 上方寫實作**

```rust
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

/// character pack artifact 的 kind 常數。
pub const KIND_CHARACTER_PACK: &str = "mori.character-pack";

/// 一個可在 body part 之間 handoff 的 artifact 的 metadata。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoriArtifact {
    pub artifact_id: String,
    /// 開放詞彙的 kind,例如 `mori.character-pack`。
    pub kind: String,
    pub path: String,
    pub visibility: Visibility,
    pub mime: String,
    pub suggested_actions: Vec<SuggestedAction>,
}

/// 資料可見度。對應 body-interface 的 data policy。BI-0 只用到 `Local`,
/// 其餘三層是鎖定契約的一部分,先列出(schema 保留,非 build)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Visibility {
    Local,
    Public,
    Internal,
    Private,
}

/// Mori 對這個 artifact 建議可做的動作。BI-0 只有 character pack 的三個。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SuggestedAction {
    Validate,
    Import,
    Activate,
}

impl MoriArtifact {
    /// 產生帶新 id 的 artifact envelope。
    pub fn new(
        kind: impl Into<String>,
        path: impl Into<String>,
        visibility: Visibility,
        mime: impl Into<String>,
        suggested_actions: Vec<SuggestedAction>,
    ) -> Self {
        Self {
            artifact_id: format!("artifact_{}", Uuid::new_v4().simple()),
            kind: kind.into(),
            path: path.into(),
            visibility,
            mime: mime.into(),
            suggested_actions,
        }
    }
}
```

- [ ] **Step 5: 加 `pub mod body;` 到 `crates/mori-core/src/lib.rs`**

在 `:14`（`pub mod annuli;`）後插入：

```rust
pub mod body;
```

- [ ] **Step 6: 跑測試確認 pass**

Run: `cargo test -p mori-core --lib body::artifact`
Expected: 2 個 test PASS。

- [ ] **Step 7: Commit**

```bash
git add crates/mori-core/src/body crates/mori-core/src/lib.rs
git commit -m "feat(bi-0): MoriArtifact semantic envelope type in mori-core"
```

---

## Task 2: `classify_artifact` 分類器

**Files:**
- Modify: `crates/mori-core/src/body/artifact.rs`(同檔加函式 + 測試）

- [ ] **Step 1: 在 test mod 加 failing test**

加進 `mod tests`：

```rust
    #[test]
    fn classify_recognizes_moripack_zip() {
        let a = classify_artifact(Path::new("/home/u/Downloads/mori.moripack.zip")).unwrap();
        assert_eq!(a.kind, KIND_CHARACTER_PACK);
        assert_eq!(a.visibility, Visibility::Local);
        assert_eq!(a.mime, "application/zip");
        assert_eq!(a.path, "/home/u/Downloads/mori.moripack.zip");
        assert!(a.suggested_actions.contains(&SuggestedAction::Activate));
    }

    #[test]
    fn classify_recognizes_plain_zip_and_moripack_ext() {
        assert!(classify_artifact(Path::new("/x/pack.zip")).is_some());
        assert!(classify_artifact(Path::new("/x/pack.moripack")).is_some());
    }

    #[test]
    fn classify_rejects_unknown_extension() {
        assert!(classify_artifact(Path::new("/x/notes.txt")).is_none());
        assert!(classify_artifact(Path::new("/x/no-extension")).is_none());
    }
```

注意：`Path` 已在實作區 `use std::path::Path;`，test mod 的 `use super::*;` 會帶進來。

- [ ] **Step 2: 跑測試確認 fail**

Run: `cargo test -p mori-core --lib body::artifact`
Expected: 編譯失敗 —「cannot find function `classify_artifact`」。

- [ ] **Step 3: 在 `impl MoriArtifact` 下方寫實作**

```rust
/// 看一個本機檔案路徑,判斷 Mori 認不認得它、能對它做什麼。
/// 認得 → artifact envelope;不認得 → `None`。
///
/// BI-0 只認 character pack(`.moripack.zip` / `.moripack` / `.zip`)。
/// 真正的內容驗證仍在 import 時做(manifest / required sprites / zip-slip)。
pub fn classify_artifact(path: &Path) -> Option<MoriArtifact> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    if name.ends_with(".moripack.zip") || name.ends_with(".moripack") || name.ends_with(".zip") {
        return Some(MoriArtifact::new(
            KIND_CHARACTER_PACK,
            path.to_string_lossy().into_owned(),
            Visibility::Local,
            "application/zip",
            vec![
                SuggestedAction::Validate,
                SuggestedAction::Import,
                SuggestedAction::Activate,
            ],
        ));
    }
    None
}
```

- [ ] **Step 4: 跑測試確認 pass**

Run: `cargo test -p mori-core --lib body::artifact`
Expected: 5 個 test 全 PASS。

- [ ] **Step 5: Commit**

```bash
git add crates/mori-core/src/body/artifact.rs
git commit -m "feat(bi-0): classify_artifact recognizes character packs"
```

---

## Task 3: `inspect_artifact` Tauri command

**Files:**
- Modify: `crates/mori-tauri/src/main.rs`(command 定義加在 character pack commands 區，約 `:2085-2182`；`invoke_handler!` 在 `:6275` 附近加一行）

> **前置確認**：main.rs 既有對 `mori_core::` 的引用(Wave 4 的 `AnnuliMemoryStore` 等)。動工前先 `grep -n "mori_core::" crates/mori-tauri/src/main.rs | head` 確認 crate 路徑寫法一致（應為 `mori_core::...`）。

- [ ] **Step 1: 加 command 定義**

加在 `character_dir`（約 `:2182`）後面：

```rust
/// BI-0:看一個本機檔案,回傳 Mori 認得的 artifact envelope(目前只有 character
/// pack)。認不得回 Err,讓 UI 顯示「Mori 不認得這個檔案」並讓使用者取消。
/// 這是 Body Interface「handoff 要可見、可取消」原則的入口。
#[tauri::command]
fn inspect_artifact(path: String) -> Result<mori_core::body::MoriArtifact, String> {
    mori_core::body::classify_artifact(std::path::Path::new(&path))
        .ok_or_else(|| format!("Mori 不認得這個檔案:{path}"))
}
```

- [ ] **Step 2: 在 `invoke_handler![...]` 註冊**

在 `:6275` 附近的 `character_dir,` 那行後面加：

```rust
            inspect_artifact,
```

- [ ] **Step 3: 編譯確認**

Run: `cargo check -p mori-tauri`
Expected: 編譯通過（command 簽名與註冊一致）。

- [ ] **Step 4: Commit**

```bash
git add crates/mori-tauri/src/main.rs
git commit -m "feat(bi-0): inspect_artifact tauri command"
```

> command 是 thin wrapper，邏輯已在 Task 2 的 `classify_artifact` 被測過；wrapper 行為在 Task 4 的 manual e2e 一起驗。

---

## Task 4: 前端 — 可見/可取消的 handoff

**Files:**
- Modify: `src/tabs/ConfigTab.tsx`（`MoriArtifact` interface + 改 `CharacterPicker` 的 `onImport` 為兩段 + handoff panel）

> 此 task 走 **integration + manual verify**：ConfigTab 目前無 vitest 測試，且流程重度依賴 Tauri `invoke`/`openDialog`（repo 對 UI 的慣例是手測，見 CLAUDE.md「Manual test 等 yazelin」）。契約邏輯已在 Rust Task 1-2 用 TDD 守住。

- [ ] **Step 1: 加 `MoriArtifact` TS interface**

在 ConfigTab.tsx 模組頂部、`MORI_SPRITE_STUDIO_URL`(約 `:40`)附近加：

```typescript
interface MoriArtifact {
  artifact_id: string;
  kind: string;
  path: string;
  visibility: string;
  mime: string;
  suggested_actions: string[];
}
```

- [ ] **Step 2: 在 `CharacterPicker` 加 pending handoff state**

在 `const [importError, setImportError] = useState<string | null>(null);`(`:513`)後加：

```typescript
  const [pending, setPending] = useState<MoriArtifact | null>(null);
```

- [ ] **Step 3: 把 `onImport`(`:547-568`)換成「pick → inspect」**

整段 `onImport` 改成 `onPickFile`：

```typescript
  const onPickFile = async () => {
    const selected = await openDialog({
      multiple: false,
      filters: [{ name: "Mori character pack", extensions: ["zip", "moripack"] }],
    });
    if (!selected || typeof selected !== "string") return;
    setImportError(null);
    try {
      const artifact = await invoke<MoriArtifact>("inspect_artifact", { path: selected });
      setPending(artifact); // 顯示可見、可取消的 handoff 確認，先不動 vault/角色
    } catch (e: any) {
      setImportError(String(e));
    }
  };

  const onConfirmImport = async () => {
    if (!pending) return;
    setImporting(true);
    setImportError(null);
    try {
      const entry = await invoke<CharacterEntry>("character_pack_import_zip", {
        zipPath: pending.path,
      });
      await refresh();
      setActive(entry.stem);
      setMsg(`✅ 已匯入:${entry.display_name} by ${entry.author}`);
      setTimeout(() => setMsg(null), 4000);
      setPending(null);
    } catch (e: any) {
      setImportError(String(e));
    } finally {
      setImporting(false);
    }
  };

  const onCancelImport = () => {
    setPending(null);
    setImportError(null);
  };
```

- [ ] **Step 4: 改匯入按鈕的 onClick + 加 handoff panel**

把 `:608-610` 的匯入按鈕 onClick 從 `onImport` 改成 `onPickFile`：

```tsx
            <button className="mori-btn" onClick={onPickFile} disabled={importing || busy}>
              {importing ? "匯入中…" : "匯入 .moripack.zip"}
            </button>
```

並在 `importError` 區塊(`:616-620`)之前插入 handoff 確認 panel：

```tsx
          {pending && (
            <div
              style={{
                border: "1px solid var(--c-border)",
                borderRadius: 8,
                padding: 10,
                display: "flex",
                flexDirection: "column",
                gap: 8,
                fontSize: 12,
              }}
            >
              <div>
                Mori 認得這個檔案:<strong>角色包</strong>(<code>{pending.kind}</code>)
              </div>
              <div style={{ opacity: 0.8 }}>
                可見度:{pending.visibility} · 可做:{pending.suggested_actions.join(" → ")}
              </div>
              <div style={{ opacity: 0.6, wordBreak: "break-all" }}>{pending.path}</div>
              <div style={{ display: "flex", gap: 8 }}>
                <button className="mori-btn" onClick={onConfirmImport} disabled={importing}>
                  {importing ? "匯入中…" : "確認匯入並套用"}
                </button>
                <button className="mori-btn ghost" onClick={onCancelImport} disabled={importing}>
                  取消
                </button>
              </div>
            </div>
          )}
```

- [ ] **Step 5: build + 既有測試不破**

Run: `npm run build && npm test`
Expected: build 成功；vitest 既有 `shellTabs.test.ts` / `FloatingMori.test.ts` 全綠（無回歸）。

- [ ] **Step 6: Manual verify(BI-0 e2e — happy path)**

```bash
npm run tauri dev
```
1. 開 Config → Floating/Appearance section 找到角色匯入。
2. 點「匯入 .moripack.zip」→ 選一個 Sprite Studio 匯出的 `.moripack.zip`。
3. **預期出現 handoff panel**：顯示「角色包(mori.character-pack)· 可做:validate → import → activate · <path>」+「確認匯入並套用 / 取消」。
4. 點「取消」→ panel 消失,**角色沒變**(驗 handoff 可取消)。
5. 重來,點「確認匯入並套用」→ 匯入成功訊息 → floating Mori sprite 即時 reload 成新角色(驗既有 `character-changed` 仍 work)。
6. 選一個非 zip 檔(若 dialog filter 擋住,可暫時放寬 filter 測 `inspect_artifact` 的 Err 路徑)→ 預期顯示「Mori 不認得這個檔案」,不進匯入。

- [ ] **Step 7: Commit**

```bash
git add src/tabs/ConfigTab.tsx
git commit -m "feat(bi-0): visible/cancellable character-pack handoff via inspect_artifact"
```

---

## Task 5: 文件收尾 + backlog 進度

**Files:**
- Modify: `docs/moripack-integration.md`(§下一步 L327-330）
- Modify: `docs/body-interface-backlog.md`(BI-0 row + 附 manual e2e）

- [ ] **Step 1: 標 moripack 文件的 Phase 1 進度**

在 `docs/moripack-integration.md` §下一步(L325 起)上方加一段狀態：

```markdown
## 實作進度(BI-0)

- ✅ Phase 1 manual artifact handoff(Open Studio / Import / Validate / Activate / reload)— #107 已落地。
- ✅ Artifact envelope 正式化:`mori_core::body::MoriArtifact` + `classify_artifact` + `inspect_artifact` command;匯入前顯示可見、可取消的 handoff(BI-0)。
- ⏳ Phase 2 custom URL(`mori://character-pack/import`)、Phase 3 local API — 未做(YAGNI,等需求)。
```

- [ ] **Step 2: 更新 backlog BI-0 row 狀態**

在 `docs/body-interface-backlog.md` §4 表格的 BI-0 row 後，於 §4 末尾「各 stage 的完成判準」區把 **BI-0 done** 那條改成已勾：

```markdown
- **BI-0 done** ✅(2026-05-27)= 能從 Appearance 開 Studio → 匯入 zip → 驗證 → 套用 → reload;且匯入前走正式 `MoriArtifact` envelope（`inspect_artifact`），handoff 可見可取消。
```

- [ ] **Step 3: Commit**

```bash
git add docs/moripack-integration.md docs/body-interface-backlog.md
git commit -m "docs(bi-0): mark MoriPack artifact handoff done + backlog progress"
```

---

## Self-Review

**1. Spec coverage**（對 `moripack-integration.md` + backlog BI-0）：
- Artifact Contract（artifact_id/kind/path/visibility/mime/suggested_actions）→ Task 1 ✓
- 認得 `.moripack.zip` → Task 2 ✓
- Desktop flow：開 Studio(已存在)/ Import / Validate / Activate / reload → 既有 + Task 4 confirm ✓
- 「handoff 要可見、可取消」(body-interface §481)→ Task 4 handoff panel + 取消 ✓
- 「匯入流程內部走正式 artifact envelope」(backlog 完成判準)→ Task 3 `inspect_artifact` 回 `MoriArtifact`，Task 4 前端據它走 ✓
- 本地驗證不信任外部 Studio（zip-slip / manifest / required sprites）→ 既有 `character_pack::import_zip` 不動，仍生效 ✓

**2. Placeholder scan:** 無 TBD / 「適當處理」/ 空 test。每個 code step 都有完整 code。✓
（唯一外部依賴：Task 3 的 `mori_core::` 路徑寫法 — 已加前置 grep 確認步驟，非 placeholder。）

**3. Type consistency:** `MoriArtifact` 欄位（Rust snake_case ↔ TS interface 同名）一致；`Visibility`/`SuggestedAction` 走 `#[serde(rename_all="lowercase")]` ↔ TS 端用 `string` 收 lowercase；command 名 `inspect_artifact` 在 Rust 定義 / 註冊 / 前端 `invoke` 三處一致；既有 `character_pack_import_zip` 參數 `zipPath`、回傳 `CharacterEntry` 沿用不變。✓

**4. 範圍紀律複查:** 沒有 generic dispatcher / registry / manifest reader / permission broker / 非 HTTP transport。只引入 1 個 semantic 型別 + 1 個分類器 + 1 個 thin command + 前端可見 handoff。符合 D1 與 backlog §1。✓

---

## 全套驗證(收工前)

```bash
bash scripts/verify.sh
```
預設涵蓋:`npm run build` / `npm test`(vitest)/ `cargo test -p mori-core --lib`(含 `body::artifact` 5 test)/ `cargo test -p mori-tauri --bin mori-tauri` / `cargo check --workspace --all-targets`。

---

**Plan saved:** `docs/superpowers/plans/2026-05-27-bi-0-moripack-artifact-handoff.md`
**Branch 建議:** `feat/bi-0-artifact-handoff`(對齊 repo 慣例)
