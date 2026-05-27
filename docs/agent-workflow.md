# Cloud Agent Workflow

This repo is prepared for subscription-based cloud coding agents such as Codex Cloud and Claude Code. The default workflow does not require OpenAI or Anthropic API keys in GitHub Actions.

## Goal

Use cloud agents to do implementation work while GitHub Actions remains the shared verification gate:

1. Create an issue, spec, or agent task.
2. Run Codex Cloud or Claude Code against this repository.
3. Let the agent create or update a pull request.
4. GitHub Actions runs `Check`, which calls `bash scripts/verify.sh`.
5. Use Codex/Claude follow-ups to address CI failures or review comments.
6. Human reviews the final diff and merges.

## Repository Contracts

Agents should read these files before editing:

- `AGENTS.md`: shared rules for Codex Cloud and other general agents.
- `CLAUDE.md`: Claude Code-specific context and Mori project constraints.
- `scripts/verify.sh`: single verification entrypoint for local dev, CI, and cloud agents.
- `.github/pull_request_template.md`: expected PR summary, verification, and platform impact notes.
- `docs/release-format.md`: unified template + section-by-section guidance for GitHub Release body (rewrite the draft Release body after `git tag` push + release.yml; do not paste CHANGELOG verbatim).

Do not let agents invent new verification commands unless a task specifically requires it. Keep the shared verifier as the source of truth.

## Codex Cloud Setup

Use Codex through your ChatGPT Team/Business subscription and connect it to GitHub from Codex web. Do not add `openai/codex-action` unless you intentionally want an API-key-backed GitHub Action.

Recommended Codex environment setup script:

```bash
sudo bash scripts/install-linux-deps.sh
npm ci
```

Canonical Ubuntu apt packages for Codex Cloud image/dependency provisioning are
listed one-per-line in `scripts/linux-build-packages.txt`:

```text
libwebkit2gtk-4.1-dev
libssl-dev
libayatana-appindicator3-dev
librsvg2-dev
libsoup-3.0-dev
libjavascriptcoregtk-4.1-dev
libasound2-dev
ffmpeg
pkg-config
build-essential
cmake
curl
wget
file
```

If the Codex environment does not allow `sudo`, configure the environment image
with the packages from `scripts/linux-build-packages.txt` outside the task setup,
then keep setup to:

```bash
npm ci
```

Recommended maintenance script for cached environments:

```bash
npm ci
```

Recommended task instruction:

```text
Read AGENTS.md and CLAUDE.md first. Keep changes narrow. Before opening the PR, run bash scripts/verify.sh and report the result. Do not reformat unrelated files. Call out Windows or Linux native behavior if touched.
```

## Claude Code Setup

Use Claude Code with the connected GitHub repository and let it read `CLAUDE.md`. For tasks that may be handled by either Claude or Codex, also point it at `AGENTS.md`.

Recommended task instruction:

```text
Read CLAUDE.md and AGENTS.md first. Implement the smallest change that satisfies the task. Run bash scripts/verify.sh before handing off. If strict formatting fails only on pre-existing files, report that instead of mass-formatting unrelated code.
```

Only use Claude Code GitHub Actions if you have intentionally provisioned `ANTHROPIC_API_KEY`, Bedrock, or Vertex AI credentials. Subscription-based Claude Code web usage does not need this repo to store an Anthropic API key.

## Pull Request Rules

Every agent-created PR should include:

- What changed.
- Why the change was made.
- Verification result from `bash scripts/verify.sh`.
- Whether Windows behavior was touched.
- Whether Linux/Tauri native behavior was touched.

Agents should not:

- Commit secrets, API keys, private vault content, or local machine paths.
- Push release tags.
- Publish GitHub Releases.
- Reformat unrelated Rust or TypeScript files.
- Rewrite docs with external project comparisons.

## CI Failure Loop

When `Check` fails:

1. Open the failing GitHub Actions run.
2. Copy the relevant failing step and error into a Codex/Claude follow-up.
3. Ask the same agent to update the existing PR branch when possible.
4. If the agent cannot update the existing branch, create a small follow-up PR that targets the original branch.
5. Re-run `Check`.

Do not introduce automatic CI-fix GitHub Actions until the normal PR review flow is stable. Auto-fix should be a later workflow that opens a separate `ai/autofix-*` PR instead of directly pushing to feature branches.

## Release Boundary

Release automation remains tag-driven through `.github/workflows/release.yml`.

Agents may prepare release notes or changelog edits, but should not create tags or publish releases unless explicitly asked by the repository owner.

Before tagging a release, run:

```bash
bash scripts/verify.sh
```

Then push a `v*` tag to trigger the draft GitHub Release build.
