---
description: Cut a patch / minor release for mori-desktop — bump version in Cargo.toml / tauri.conf.json / package.json, append CHANGELOG entry, sync roadmap, then commit. Invoke when the user says "發佈 vX.Y.Z" / "release vX.Y.Z" / "cut a patch release" / "出個小 release". Does NOT push or tag — stops at the local commit and reports next steps so user picks A (direct push) vs B (PR flow).
---

# release-patch — Mori-desktop release commit

Cut a release commit locally. **Stops before push / tag** so the user makes the final call(direct push 還是走 PR 都要他決定)。

## When to invoke

User says any of:
- "發佈 vX.Y.Z" / "release vX.Y.Z" / "cut a patch / minor release"
- "出個 hotfix release" / "ship a patch"
- Implied:剛改完 bug 想正式發,還在問怎麼 release

不適用:major version bump (v1.0.0 等大改) → 那層需要 migration guide / breaking change notes 之類更多討論,別跑自動流程。

## Pre-flight checks

1. **Working tree** — 跑 `git status`,確認:
   - 在 `main` branch(或 user 明確要 release 的 branch)
   - 沒未 staged 的 fix 殘留(若有應該先 commit 進另一個 `fix(...):` commit)
   - 注意 `examples/scripts/__pycache__/` 之類本機 byproduct,**別 stage 進 release commit**
2. **新版號** — 從 user 訊息抓(`v0.7.1` / `0.7.1`)或讀 `Cargo.toml` `[workspace.package].version` +1 patch。問清楚不要猜。
3. **想 release 什麼?** — `git log origin/main..HEAD --oneline` 看待 release 的 commits,跟 user 對清楚 release notes 該寫什麼。

## Steps

### 1. Bump version in 3 places

跨平台都是純文字編輯,Edit tool 即可:

| 檔 | 找這行 | 改成 |
|---|---|---|
| `Cargo.toml` | `[workspace.package]` 區的 `version = "X.Y.Z"` | 新版號 |
| `crates/mori-tauri/tauri.conf.json` | `"version": "X.Y.Z",` | 新版號 |
| `package.json` | `"version": "X.Y.Z",` | 新版號 |

### 2. Refresh Cargo.lock

```bash
cargo check -p mori-tauri --message-format=short 2>&1 | tail -5
```

只跑 check 不 build。version bump 會自動寫進 Cargo.lock。**期待:** 沒新錯,既有 `unused import: CommandExt` warning(`annuli_supervisor.rs`)是 pre-existing,忽略。

### 3. CHANGELOG entry

Prepend 進 `CHANGELOG.md`,**在 `## v<上一版>` 之前**,結構:

```markdown
## v<版本> — <一句 tagline,跟 release body 同調>(<YYYY-MM-DD>)

<Hook 2-3 句:前一版的什麼問題 / 缺口 → 這版解了什麼。讀者是 future-yazelin / contributor reviewer。>

### <子主題 1 — feature 或 fix area>

<細節描述。比 release body 詳,寫得到「為什麼這樣解」「dev 模式 vs release 差異」「相關 PR / commit」這層>

- ...

### <子主題 2>

...

### Verified

實機跑通的 user-facing scenario,3-8 條:
- ✅ ...

### 升級

無 breaking change / migration 細節 / config schema diff。沒就一行寫「無 breaking change」。

---
```

風格參考最近 v0.7.1 / v0.7.0 entries — 中英混、技術細節到位、不寫廢話。

### 4. Sync roadmap

讀 `docs/roadmap.md`,找有沒有「該版本 ship 完後變 stale 的條目」:
- 該標 ✅ 還沒標的
- 「待補 / TODO」清單該移除的條目
- 「目前不能 X」對的 X 已經能

修法:就地 edit 加 ✅ / 改描述,別整段刪(歷史脈絡留著)。

### 5. Stage + commit

```bash
git add Cargo.toml crates/mori-tauri/tauri.conf.json package.json CHANGELOG.md docs/roadmap.md Cargo.lock
git diff --cached --stat   # 確認只動到這幾檔,沒誤 stage 別的
git commit -m "$(cat <<'EOF'
release: v<X.Y.Z> — <tagline,跟 CHANGELOG 標題一致>

<2-4 行說明:這次 release 主要修了什麼。比 CHANGELOG section header
更摘要,但比 commit subject 多一點 context。讀者是 git log 看 commit
history 的人。>

CHANGELOG 完整列改動,簡述:

- <bullet 1 - feature / fix area>
- <bullet 2>
- ...

<其他要 surface 的 — roadmap 更新 / breaking change 警告 / closes #N>

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

`git commit` 用 HEREDOC 喂 message 是為了正確處理多行 + 不被 shell escape 吃掉。Git Bash on Windows 跟 Linux bash 都吃這 syntax。

### 6. Report to user, stop

`git log --oneline -3` 給 user 看新 commit 上去了。然後**明確告訴 user 兩條 push 路徑**:

```
v<X.Y.Z> commit 在 local 了,還沒 push。

A. 直推 main + tag(快):
   git push origin main
   git tag v<X.Y.Z>
   git push origin v<X.Y.Z>
   → 觸發 .github/workflows/release.yml,CI build Linux + Windows,draft release。

B. 走 PR(repo 慣例,過去 release commit 都有 (#N) 後綴):
   git checkout -b release/v<X.Y.Z>
   git push origin release/v<X.Y.Z>
   gh pr create --title "release: v<X.Y.Z> — <tagline>" --body "<CHANGELOG section>"
   → merge 後手動 tag merge commit + push tag。

你選哪條?
```

**不**要自己選或自己 push — push / tag 是 destructive remote action,要 user 顯式同意。

## Cross-platform notes

- 所有 shell 指令用 Git Bash 跟 Linux bash 都認的 syntax(`$(cat <<'EOF' ... EOF)` / forward slash paths)。Windows native cmd / PowerShell 別跑這些 command,要用 Bash tool。
- `cargo check` 跨平台同個指令。
- `git` 行為相同。

## 失敗 recovery

- **`cargo check` fail** — 通常是版號 bump 後某 crate.toml 還沒更新,或 release commit 試圖加新 feature。停下來修,別繼續往 commit 走。
- **CHANGELOG / roadmap edit fail** — 多半是 Edit 的 `old_string` 抓不準,改用 Read 看現況。
- **`git commit` fail** — pre-commit hook 失敗的話**新建一個 fix commit** 別 `--amend`(避免動到還沒寫的 commit 內容)。
