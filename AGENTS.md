# Agent Instructions - mori-desktop

This file is the shared operating guide for Codex Cloud, local Codex, and other coding agents. Claude-specific context also lives in `CLAUDE.md`; keep both files aligned when changing project workflow.

## Project Shape

mori-desktop is a Tauri 2 desktop app with a Vite/React/TypeScript frontend and a Rust workspace:

- `src/`: frontend UI
- `crates/mori-core/`: core agent, LLM, memory, URL, voice cleanup logic
- `crates/mori-cli/`: CLI helper used by npm prebuild hooks
- `crates/mori-tauri/`: Tauri shell, native commands, skills, platform integration
- `scripts/`: setup and verification scripts used by humans, CI, and agents
- `docs/agent-workflow.md`: cloud-agent SOP for Codex Cloud, Claude Code, PRs, CI, and releases
- `docs/release-format.md`: unified template for GitHub Release body (publish step after `git tag` + release.yml). Follow it instead of pasting CHANGELOG — different audience.

The repository is part of the Mori universe. Preserve the existing poetic Traditional Chinese product language in docs/UI, but keep code identifiers conventional Rust/TypeScript unless nearby code already uses Mori-specific terms.

## Hard Rules

- Do not write public comparisons such as "vs OpenHuman" or "inspired by Hermes Agent" in README, roadmap, PR body, release notes, or docs.
- User data stays user-owned. Do not introduce central OAuth relays, SaaS hubs, or third-party data brokers.
- Do not write to `mori-journal` identity or memory files if that private repo is present. Only write under `projects/` after explicit authorization.
- Keep setup dependencies in this repo. Use `scripts/install-linux-deps.sh`; do not fetch setup logic from another repository.
- annuli integration should go over HTTP, not direct Python imports.
- Keep changes narrow. Do not reformat unrelated files or rewrite large surfaces unless the task explicitly requires it.

## Setup

On Ubuntu/Linux runners, install native Tauri dependencies first:

```bash
sudo bash scripts/install-linux-deps.sh
```

Install Node dependencies:

```bash
npm ci
```

`npm run build` invokes `prebuild`, which builds `mori-cli` in release mode before TypeScript/Vite. Do not remove this hook unless replacing it with an equivalent build path.

## Verification

Use the shared verifier before opening or updating a PR:

```bash
bash scripts/verify.sh
```

The default verifier runs:

- `npm run build`
- `cargo test -p mori-core --lib`
- `cargo check --workspace --all-targets`

Strict style checks are available but not yet CI-blocking because the existing Rust tree is not fully rustfmt-clean:

```bash
VERIFY_STRICT=1 bash scripts/verify.sh
```

If strict mode fails only on pre-existing formatting or clippy findings outside your change, mention that in the PR instead of mass-formatting unrelated files.

## Platform Notes

- CI must keep both Ubuntu and Windows compile coverage.
- Windows command/path behavior is fragile. Check `mori.exe` explicitly where relevant; `PathBuf::exists()` will not append `.exe`.
- On Windows, prefer native shell APIs already used by the repo over `cmd /c start`.
- `HOME` may be unset on Windows; preserve existing `USERPROFILE` fallbacks.
- Linux native dependencies for Tauri/WebKit/ALSA come from `scripts/install-linux-deps.sh`.

## Pull Requests

Before creating a PR:

- Read `docs/agent-workflow.md` if you are running as a cloud agent.
- Run `bash scripts/verify.sh`, or clearly state why it could not run.
- Summarize behavior changes and platform impact.
- Call out whether Windows behavior was touched.
- Keep generated or formatting-only churn out of feature PRs.
- Do not include secrets, API keys, local vault content, or private Mori journal content.
