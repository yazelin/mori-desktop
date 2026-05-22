# Floating Cross-Platform Backdrop Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let character packs ship their own backdrop image, with a user-controlled on/off toggle that works on X11, Wayland, and Windows.

**Architecture:** Move the backdrop from a body-level background (X11 only, blocked by `body { background: transparent !important }` on other platforms) to a DOM element (`<div class="mori-backdrop">`) inside the floating window. The image is resolved through a three-step chain: character pack `backdrop-{dark,light}.png` → user global `~/.mori/floating/backplate-{dark,light}.png` → shipped default. The `floating.x11_backplate` config key is renamed to `floating.backplate` (value strings `"plain"` / `"logo"` unchanged) with dual-read backwards-compat.

**Tech Stack:** Tauri 2, React 19, TypeScript, Rust (serde / base64 / fs).

**Scope decisions:**
- **Keep the existing X11 `"plain"` gradient** (body background under `body.x11-fallback`). It's defensive against the X11 + WebKit2GTK half-alpha render bug for users with soft sprite effects. The new DOM backdrop only kicks in for `"logo"` mode, which is what gains cross-platform support.
- **Keep the `x11-fallback` class itself** (still used for the border-ring `::before`, sprite-area centering, info-label positioning — all unchanged).
- **Filename convention, not manifest field**: character packs drop `backdrop-{dark,light}.png` in the pack root. No schema_version bump needed.
- **Two image variants (dark/light)**, mirroring the existing user-global convention.

---

## File Map

**Modify:**
- `crates/mori-tauri/src/main.rs` — add `read_character_backdrop` IPC, de-gate `read_floating_backplate` from Linux-only.
- `src/FloatingMori.tsx` — replace `applyX11Backplate` with `applyBackdrop` (three-step resolution chain), add `activeStem` state, render `<div className="mori-backdrop">` element.
- `src/floating.css` — add `.mori-backdrop` rules, remove obsolete `body.x11-fallback.backplate-logo` rules and the light-theme variant.
- `src/tabs/ConfigTab.tsx` — relabel "x11 backplate" → "backplate", read/write new key with old-key fallback.
- `src/i18n/locales/zh-TW.json`, `src/i18n/locales/en.json` — rename `hint_x11_backplate` → `hint_backplate`, update hint text.
- `config.example.json` — rename key `x11_backplate` → `backplate`.
- `docs/character-pack.md` — add "Backdrop (optional)" section.

**No new files.** No new dependencies.

---

## Task 1: Add `read_character_backdrop` IPC (with test)

**Files:**
- Modify: `crates/mori-tauri/src/main.rs:531-560` (next to `read_floating_backplate`)
- Modify: `crates/mori-tauri/src/main.rs:5360` (command registration)
- Test: `crates/mori-tauri/src/main.rs` (inline `#[cfg(test)]` module at bottom — see Step 1 location below)

- [ ] **Step 1: Add the IPC command**

After `read_floating_backplate` (currently ends ~line 560), insert:

```rust
/// 讀 character pack 的 `~/.mori/characters/<stem>/backdrop-{dark,light}.png`,
/// 有的話以 base64 data URL 回給 React。沒有就 Ok(None) — React 端 fallback
/// 到 user global,再 fallback 到 shipped default。
///
/// 跟 `read_floating_backplate` 並列(後者讀 user global `~/.mori/floating/`),
/// 拆兩支 command 是因為 input 不同(stem+theme vs theme),分開比加 Option<stem>
/// 分支清楚。
#[tauri::command]
fn read_character_backdrop(stem: String, theme: String) -> Result<Option<String>, String> {
    use base64::Engine as _;
    if !matches!(theme.as_str(), "dark" | "light") {
        return Err(format!(
            "invalid theme '{theme}', expected 'dark' or 'light'"
        ));
    }
    let path = crate::character_pack::pack_dir(&stem)
        .join(format!("backdrop-{theme}.png"));
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(Some(format!("data:image/png;base64,{b64}")))
}
```

- [ ] **Step 2: Register the command**

In the `tauri::generate_handler!` list around `crates/mori-tauri/src/main.rs:5360`, add `read_character_backdrop,` next to `read_floating_backplate,`. The line to find is:

```rust
            read_floating_backplate,
```

Change to:

```rust
            read_floating_backplate,
            read_character_backdrop,
```

- [ ] **Step 3: Write a unit test for path + theme validation**

At the bottom of `crates/mori-tauri/src/main.rs`, look for an existing `#[cfg(test)] mod tests` block. If one exists, append; otherwise create at end of file:

```rust
#[cfg(test)]
mod backdrop_ipc_tests {
    use super::*;

    #[test]
    fn read_character_backdrop_rejects_unknown_theme() {
        let err = read_character_backdrop("mori".into(), "neon".into()).unwrap_err();
        assert!(err.contains("invalid theme"), "got: {err}");
    }

    #[test]
    fn read_character_backdrop_missing_file_returns_none() {
        // Use a stem that almost certainly has no backdrop-dark.png in test env
        let out = read_character_backdrop("__nonexistent_pack__".into(), "dark".into()).unwrap();
        assert!(out.is_none(), "expected None for missing file, got Some");
    }
}
```

- [ ] **Step 4: Run cargo check + tests**

Run: `cd crates/mori-tauri && cargo test backdrop_ipc_tests -- --nocapture`
Expected: 2 tests pass. If `cargo test` fails because the binary crate has GUI deps that won't link in test mode, fall back to: `cargo check -p mori-tauri` (just verify it compiles). Note the actual outcome.

- [ ] **Step 5: De-gate `read_floating_backplate` from Linux-only**

In `crates/mori-tauri/src/main.rs:531-560`, replace the entire function body with the un-gated version using `mori_dir()` (which already handles HOME/USERPROFILE cross-platform):

```rust
#[tauri::command]
fn read_floating_backplate(theme: String) -> Result<Option<String>, String> {
    use base64::Engine as _;
    if !matches!(theme.as_str(), "dark" | "light") {
        return Err(format!(
            "invalid theme '{theme}', expected 'dark' or 'light'"
        ));
    }
    let path = crate::mori_dir()
        .join("floating")
        .join(format!("backplate-{theme}.png"));
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(Some(format!("data:image/png;base64,{b64}")))
}
```

Also update the doc comment above (line ~484-495) to remove "X11" mentions; replace with:

```rust
/// 讀使用者全域 `~/.mori/floating/backplate-{dark,light}.png`,有的話以 base64
/// data URL 回給 React。沒有就 Ok(None),React 端 fallback 到 shipped default。
///
/// 用 data URL 而不是 Tauri asset protocol:asset protocol 需要 in tauri.conf.json
/// security 開啟 + 設 scope。data URL 直接是字串,React → CSS variable → background-image
/// 一條龍,不動 Tauri 設定。檔案 ~500KB,base64 ~700KB,記憶體 OK。
```

- [ ] **Step 6: Run cargo check**

Run: `cd crates/mori-tauri && cargo check`
Expected: compiles clean (warnings OK, no errors).

- [ ] **Step 7: Commit**

```bash
git add crates/mori-tauri/src/main.rs
git commit -m "feat(floating): add read_character_backdrop IPC, de-gate read_floating_backplate

- New command reads ~/.mori/characters/<stem>/backdrop-{dark,light}.png
- read_floating_backplate now works on all platforms (uses mori_dir helper)
- Tests cover theme validation and missing-file Ok(None) path"
```

---

## Task 2: Add backdrop resolution helper + state in `FloatingMori.tsx`

**Files:**
- Modify: `src/FloatingMori.tsx:158-192` (replace `applyX11Backplate` with new helper)
- Modify: `src/FloatingMori.tsx:228-291` (add `activeStem` state, change effects)

- [ ] **Step 1: Replace the `applyX11Backplate` function**

Delete the entire block from `src/FloatingMori.tsx:158-192` (the comment + function) and replace with:

```ts
type BackplateMode = "plain" | "logo";

/**
 * 解析 backdrop 圖片 chain(高優先到低):
 * 1. character pack 自己的 ~/.mori/characters/<stem>/backdrop-{theme}.png
 * 2. user 全域 ~/.mori/floating/backplate-{theme}.png
 * 3. shipped fallback(CSS var 預設 url(...))
 *
 * 任一階成功就直接 return data URL,失敗(網路 invoke 例外 / 檔不存在)往下走。
 */
async function resolveBackdropUrl(
  stem: string,
  theme: "dark" | "light",
): Promise<string | null> {
  try {
    const url = await invoke<string | null>("read_character_backdrop", { stem, theme });
    if (url) return url;
  } catch (e) {
    console.warn(`[FloatingMori] read_character_backdrop ${theme} failed`, e);
  }
  try {
    const url = await invoke<string | null>("read_floating_backplate", { theme });
    if (url) return url;
  } catch (e) {
    console.warn(`[FloatingMori] read_floating_backplate ${theme} failed`, e);
  }
  return null;
}

/**
 * Backdrop 模式套用(跨平台):
 * - "plain" → 清空 CSS variables,.mori-backdrop 元素 background-image 變 none
 * - "logo"  → 跑 resolveBackdropUrl 拿 dark + light data URL,寫進 CSS variables
 *
 * X11 plain 模式的不透明 gradient body bg(body.x11-fallback)還是有效,
 * 那是另一套(防 WebKit half-alpha bug),不在這裡管。
 */
async function applyBackdrop(mode: BackplateMode, stem: string) {
  const root = document.documentElement;
  if (mode !== "logo") {
    root.style.removeProperty("--mori-backdrop-dark");
    root.style.removeProperty("--mori-backdrop-light");
    return;
  }
  for (const theme of ["dark", "light"] as const) {
    const dataUrl = await resolveBackdropUrl(stem, theme);
    if (dataUrl) {
      root.style.setProperty(`--mori-backdrop-${theme}`, `url(${dataUrl})`);
    } else {
      root.style.removeProperty(`--mori-backdrop-${theme}`);
    }
  }
}
```

- [ ] **Step 2: Add `activeStem` and `backplateMode` state**

Just after `const [manifest, setManifest] = useState<CharacterManifest | null>(null);` (around line 229), add:

```ts
const [activeStem, setActiveStem] = useState<string>("mori");
const [backplateMode, setBackplateMode] = useState<BackplateMode>("plain");
```

- [ ] **Step 3: Wire `setActiveStem` in `loadCharacterPack`**

In `src/FloatingMori.tsx:294-324`, find:

```ts
const [stem, m] = await invoke<[string, CharacterManifest]>("character_get_active");
setManifest(m);
```

Change to:

```ts
const [stem, m] = await invoke<[string, CharacterManifest]>("character_get_active");
setActiveStem(stem);
setManifest(m);
```

- [ ] **Step 4: Replace `applyX11Backplate` call with `setBackplateMode` in `loadFloatingConfig`**

In `src/FloatingMori.tsx:248-291`, find:

```ts
// X11 backplate 動態套用(Wayland body 透明,sub-rule 不會匹配,沒副作用)
const backplate = parsed?.floating?.x11_backplate ?? "plain";
await applyX11Backplate(backplate);
```

Change to:

```ts
// 跨平台 backplate 模式(dual-read：新 key backplate 優先,fallback 舊 key x11_backplate)
const backplate: BackplateMode =
  (parsed?.floating?.backplate ?? parsed?.floating?.x11_backplate ?? "plain") as BackplateMode;
setBackplateMode(backplate);
```

- [ ] **Step 5: Add effect that applies backdrop on mode/stem change**

After the `useEffect` that ends at `src/FloatingMori.tsx:331` (the `loadCharacterPack` one), add a new effect:

```ts
// 模式或角色變動就重套 backdrop。stem 也是 dep — 切角色時即便 mode 不變,
// character pack 自帶的 backdrop 也要重抓。
useEffect(() => {
  applyBackdrop(backplateMode, activeStem);
}, [backplateMode, activeStem]);
```

- [ ] **Step 6: Run typecheck**

Run: `npx tsc -b --noEmit`
Expected: no errors.

- [ ] **Step 7: Commit**

```bash
git add src/FloatingMori.tsx
git commit -m "feat(floating): backdrop resolution chain + activeStem state

Adds resolveBackdropUrl (character pack → user global → fallback) and
applyBackdrop that runs on mode/stem change. New backplateMode + activeStem
state replace the imperative applyX11Backplate call site."
```

---

## Task 3: Add `<div class="mori-backdrop">` DOM element + CSS

**Files:**
- Modify: `src/FloatingMori.tsx:709-722` (insert backdrop element)
- Modify: `src/floating.css` (add `.mori-backdrop` rules, after `.mori-sprite-area` block ~line 163)

- [ ] **Step 1: Add the DOM element**

In `src/FloatingMori.tsx:709-722`, find:

```tsx
<div className="mori-sprite-area">
  {/* 背景光暈：錄音中由音量驅動；其他狀態 CSS animation */}
  <div className="mori-aura" style={auraStyle} />
```

Insert backdrop as the FIRST child (so it sits behind aura/sprite via DOM order):

```tsx
<div className="mori-sprite-area">
  {/* 背板:可選的角色背景圖(character pack / user global / shipped fallback) */}
  <div className="mori-backdrop" />
  {/* 背景光暈：錄音中由音量驅動；其他狀態 CSS animation */}
  <div className="mori-aura" style={auraStyle} />
```

- [ ] **Step 2: Add CSS rules**

In `src/floating.css`, after the `.mori-sprite-area` block (currently ends ~line 163, just before `/* ─── aura ─── */`), insert:

```css
/* ─── backdrop(可選背板:character pack > user global > shipped default) ─ */

/* DOM 元素,跨平台。CSS variable 從 React 餵 — applyBackdrop 跑 resolveBackdropUrl
   拿到 data URL 寫進 --mori-backdrop-{dark,light}。模式 "plain" 時 variable
   被清,background-image 變 none(無背板)。
   inset:0 蓋滿 sprite-area(160×160),也就是 sprite + drop-shadow + aura 全
   蓋住 — X11 上 logo 模式可以順便緩解 WebKit half-alpha bug(整個 region
   都是 opaque pixel)。 */
.mori-backdrop {
  position: absolute;
  inset: 0;
  pointer-events: none;
  background-image: var(--mori-backdrop-dark);
  background-position: center center;
  background-size: cover;
  background-repeat: no-repeat;
}
html[data-theme-base="light"] .mori-backdrop {
  background-image: var(--mori-backdrop-light);
}
```

- [ ] **Step 3: Run typecheck + visual sanity**

Run: `npx tsc -b --noEmit`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/FloatingMori.tsx src/floating.css
git commit -m "feat(floating): render cross-platform .mori-backdrop DOM element"
```

---

## Task 4: Remove obsolete X11 body-bg logo rules

The `.mori-backdrop` DOM element now handles the logo case on all platforms. The body-level `.backplate-logo` rules on X11 are dead code. Remove them. The `body.x11-fallback` plain gradient stays (it's the X11 default-state safety net).

**Files:**
- Modify: `src/floating.css:61-95` (delete the two `body.x11-fallback.backplate-logo` blocks)

- [ ] **Step 1: Delete the obsolete CSS rules**

In `src/floating.css`, delete lines 61-95 (everything from the `/* logo 模式 ...` comment through both dark/light `backplate-logo` selectors). For reference, the block to delete starts with:

```css
/* logo 模式(config: floating.x11_backplate = "logo"):疊一張 backplate PNG。
```

and ends after the closing `}` of:

```css
html[data-theme-base="light"] body.floating-window.x11-fallback.backplate-logo {
  background:
    ...
  ) !important;
}
```

After deletion, the surrounding context should read:

```css
body.floating-window.x11-fallback {
  /* ... existing X11 plain gradient — unchanged ... */
  background: ...;
  position: relative;
}
/* X11 floating 的 border ring + 內部光暈。... */
body.floating-window.x11-fallback::before {
  ...
}
```

(i.e., `::before` immediately follows the `body.floating-window.x11-fallback` plain block.)

- [ ] **Step 2: Verify no other references to `backplate-logo` exist**

Run: `grep -rn "backplate-logo" src/ public/ --include="*.css" --include="*.tsx" --include="*.ts" --include="*.html"`
Expected: zero matches.

- [ ] **Step 3: Commit**

```bash
git add src/floating.css
git commit -m "refactor(floating): drop dead X11 body-bg backplate-logo CSS

DOM-level .mori-backdrop replaced these on all platforms. The X11 plain
gradient (body.x11-fallback) stays as a WebKit half-alpha workaround."
```

---

## Task 5: Rename config key `x11_backplate` → `backplate` (ConfigTab + example)

**Files:**
- Modify: `src/tabs/ConfigTab.tsx:2065-2082`
- Modify: `config.example.json:86`
- Modify: `src/i18n/locales/zh-TW.json`
- Modify: `src/i18n/locales/en.json`

- [ ] **Step 1: Update ConfigTab Select binding**

In `src/tabs/ConfigTab.tsx:2065-2082`, replace:

```tsx
<FormRow
  label="x11 backplate"
  hint={t("config_tab.rows.hint_x11_backplate")}
>
  <Select
    value={cfg.floating?.x11_backplate ?? "plain"}
    onChange={(v) =>
      applyPatch((c) => {
        const f = ensureSubObj(c, "floating");
        f.x11_backplate = v;
      })
    }
    options={[
      { value: "plain", label: "素色(跟著 theme 漸層)" },
      { value: "logo", label: "背板(美術 PNG / 自訂)" },
    ]}
  />
</FormRow>
```

With (dual-read on view, single-write to new key, delete old key on save):

```tsx
<FormRow
  label="backplate"
  hint={t("config_tab.rows.hint_backplate")}
>
  <Select
    value={
      (cfg.floating?.backplate as string | undefined) ??
      (cfg.floating?.x11_backplate as string | undefined) ??
      "plain"
    }
    onChange={(v) =>
      applyPatch((c) => {
        const f = ensureSubObj(c, "floating");
        f.backplate = v;
        // 順便清掉舊 key,避免兩個並存造成困惑
        delete f.x11_backplate;
      })
    }
    options={[
      { value: "plain", label: "素色(跟著 theme 漸層)" },
      { value: "logo", label: "背板(角色 / 自訂 PNG)" },
    ]}
  />
</FormRow>
```

- [ ] **Step 2: Update `config.example.json`**

In `config.example.json:86`, replace:

```json
    "x11_backplate": "plain",
```

With:

```json
    "backplate": "plain",
```

- [ ] **Step 3: Rename + update i18n hints**

In `src/i18n/locales/zh-TW.json`, find:

```json
      "hint_x11_backplate": "floating window 內部底圖。logo 模式可放自己 PNG 在 ~/.mori/floating/backplate-{dark,light}.png 取代預設 Mori logo，即時生效。",
```

Replace with:

```json
      "hint_backplate": "floating window 背板。logo 模式優先讀 character pack 自帶的 backdrop-{dark,light}.png；沒有就讀 ~/.mori/floating/backplate-{dark,light}.png；都沒有就用內建預設。即時生效,跨平台。",
```

In `src/i18n/locales/en.json`, find:

```json
      "hint_x11_backplate": "Backplate behind the floating window contents. In logo mode, drop your own PNG at ~/.mori/floating/backplate-{dark,light}.png to replace the default Mori logo — applies immediately.",
```

Replace with:

```json
      "hint_backplate": "Backplate behind the floating window. In logo mode, the active character pack's own backdrop-{dark,light}.png wins; otherwise ~/.mori/floating/backplate-{dark,light}.png; otherwise the shipped default. Applies immediately. Cross-platform.",
```

- [ ] **Step 4: Run typecheck**

Run: `npx tsc -b --noEmit`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/tabs/ConfigTab.tsx config.example.json src/i18n/locales/zh-TW.json src/i18n/locales/en.json
git commit -m "feat(config): rename floating.x11_backplate → floating.backplate

Cross-platform now — old name was misleading. Dual-read keeps existing
configs working; ConfigTab save deletes the old key once the user touches
the row."
```

---

## Task 6: Document the backdrop convention

**Files:**
- Modify: `docs/character-pack.md` (add a section)

- [ ] **Step 1: Find the place to insert**

Open `docs/character-pack.md` and locate the section that describes the pack directory layout (likely a `## 結構` or `## Layout` block listing `manifest.json`, `sprites/`).

- [ ] **Step 2: Add a "Backdrop (optional)" subsection**

Immediately after the directory layout block, insert:

```markdown
## Backdrop(optional)

角色作者可以在 pack 根目錄(跟 `manifest.json` 同層,**不在** `sprites/` 下)
放兩張 PNG 當作角色專屬背板:

```
~/.mori/characters/<stem>/
├── manifest.json
├── sprites/...
├── backdrop-dark.png    ← optional,dark theme 時顯示
└── backdrop-light.png   ← optional,light theme 時顯示
```

兩張都是 optional;只有一張也行(對應 theme 沒檔就走下一層 fallback)。

### 顯示條件

使用者 `~/.mori/config.json` 的 `floating.backplate` 必須是 `"logo"`。`"plain"` 模式下不論角色提不提供背板都不顯示。

### Fallback chain(高優先到低)

1. character pack 自帶的 `backdrop-{dark,light}.png`
2. 使用者全域 `~/.mori/floating/backplate-{dark,light}.png`
3. 內建預設(shipped)

### 規格建議

- 尺寸:建議 320×320 或更大的方形 PNG(會 `background-size: cover` 縮放填滿 160×160 sprite-area)
- 格式:PNG-32(透明背景 OK,但這層通常做不透明 — 整個區域是 opaque 才能緩解 X11 + WebKit2GTK 的 half-alpha 渲染問題)
- 風格:留 sprite 中央區域空白 / 柔光,避免角色被背板蓋住
```

- [ ] **Step 3: Commit**

```bash
git add docs/character-pack.md
git commit -m "docs(character-pack): document optional backdrop-{dark,light}.png convention"
```

---

## Task 7: Manual verification

No automated end-to-end harness exists in this project — verify by running the floating window.

- [ ] **Step 1: Dev build**

Run: `npm run dev` (in worktree root)
Expected: Vite dev server starts; Mori floating window appears.

- [ ] **Step 2: Default state (plain mode)**

- Confirm `~/.mori/config.json` has `floating.backplate: "plain"` (or no key — defaults).
- Expected on X11: floating Mori shows the existing radial+linear gradient body bg (unchanged behavior).
- Expected on Wayland/Windows: floating Mori sits on transparent (unchanged behavior).
- No `.mori-backdrop` image visible in any case.

- [ ] **Step 3: Logo mode without character backdrop**

- Open Config tab → set `backplate` to `背板(角色 / 自訂 PNG)` and save.
- Confirm `~/.mori/floating/backplate-dark.png` and `backplate-light.png` exist (or drop a PNG there for testing).
- Expected: `.mori-backdrop` div now shows the user-global PNG, on all platforms.

- [ ] **Step 4: Logo mode with character backdrop**

- Drop a test PNG at `~/.mori/characters/mori/backdrop-dark.png`.
- Refresh the floating window (toggle backplate setting, or restart).
- Expected: the character's own PNG wins over the user-global one. Use a visually distinct test image (e.g. red square) to verify.

- [ ] **Step 5: Theme toggle**

- Switch theme between dark and light (in the app's theme tab or however it's done).
- Expected: `.mori-backdrop` swaps between `--mori-backdrop-dark` and `--mori-backdrop-light` images live (no restart).

- [ ] **Step 6: Migration test (old config key)**

- In a clean test, manually edit `~/.mori/config.json` so it has `"x11_backplate": "logo"` and NO `"backplate"` key.
- Restart app.
- Expected: floating window reads the old key correctly, logo mode works.
- Open Config tab and save anything → reopen config.json → expected: `backplate` is now present and `x11_backplate` is gone.

- [ ] **Step 7: Switch character**

- If multiple character packs are installed, switch active character via the picker.
- Expected: backdrop re-resolves (if new character has its own backdrop PNG, it appears immediately; otherwise falls back to user-global).

- [ ] **Step 8: Final cargo + tsc check**

Run in parallel (separate terminals or sequentially):
- `cargo check -p mori-tauri` → clean
- `npx tsc -b --noEmit` → clean
- `npm run build` → clean

- [ ] **Step 9: Document verification results**

Write a brief note in the PR description with: which platforms were tested (X11 / Wayland / Windows / macOS), which steps were skipped (e.g., "no Windows test available"), and any visual quirks observed.

---

## Self-Review Notes

1. **Spec coverage check:**
   - Cross-platform backdrop → Tasks 1, 2, 3 (DOM element + chain resolution + de-gated Rust IPCs). ✓
   - Per-character convention → Task 1 + Task 6 (IPC + docs). ✓
   - User on/off toggle (existing `"plain"` / `"logo"`) → Task 5 (renamed key, same semantics). ✓
   - Remove X11-specific feel from config → Task 5 (rename). ✓
   - Don't break existing configs → dual-read in Task 2 step 4 + ConfigTab in Task 5 step 1. ✓
   - X11 plain-gradient defensive workaround → explicitly kept (scope note + Task 4). ✓

2. **Placeholder scan:** none — every step has either exact code or exact commands. Manual verification steps have explicit observable expectations.

3. **Type consistency:**
   - `BackplateMode = "plain" | "logo"` defined Task 2 Step 1, used in Step 2 (`backplateMode` state) and Step 4 (cast on read). ✓
   - `resolveBackdropUrl(stem, theme)` and `applyBackdrop(mode, stem)` signatures match all call sites. ✓
   - Rust `read_character_backdrop(stem: String, theme: String)` matches the JS `invoke("read_character_backdrop", { stem, theme })` in Task 2 Step 1. ✓
   - CSS variable names `--mori-backdrop-dark` / `--mori-backdrop-light` match between Task 2 Step 1 (set) and Task 3 Step 2 (consume). ✓

4. **Edge cases handled:**
   - Both PNGs missing → `resolveBackdropUrl` returns `null` → CSS variable unset → `background-image: var(--mori-backdrop-dark)` evaluates to `var()` with no value → effectively `none`. ✓
   - One theme PNG present, other missing → only the present one sets its variable; the missing one falls through chain independently. ✓
   - Invalid theme string (e.g. `"neon"`) → Rust returns `Err`, TS catches and warns, continues to next fallback. ✓
   - Old config has `x11_backplate` only → dual-read picks it up; saving via ConfigTab migrates it. ✓
