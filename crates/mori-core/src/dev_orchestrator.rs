use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevTaskStatus {
    Queued,
    Planning,
    Executing,
    Succeeded,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevTask {
    pub id: String,
    pub prompt: String,
    pub created_at_ms: i64,
    pub status: DevTaskStatus,
    pub verify_profile: VerifyProfile,
    pub finished_at_ms: Option<i64>,
}


#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyProfile {
    None,
    Quick,
    Full,
}

impl Default for VerifyProfile {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevReport {
    pub task_id: String,
    pub summary: String,
    pub changed_files: Vec<String>,
    pub error: Option<String>,
    pub workspace_dir: String,
    pub executor_command: Option<String>,
    pub executor_ok: Option<bool>,
    pub executor_output: Option<String>,
    pub git_diff: Option<String>,
    pub verify_command: Option<String>,
    pub verify_ok: Option<bool>,
    pub verify_output: Option<String>,
    pub iteration_count: u32,
    pub budget_exhausted: bool,
    pub replay_log: Vec<String>,
    pub quality_score: u8,
}


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevTaskSnapshot {
    pub task: DevTask,
    pub report: Option<DevReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevPrDraft {
    pub task_id: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevApplyResult {
    pub task_id: String,
    pub applied_files: Vec<String>,
    pub deleted_files: Vec<String>,
    pub command: String,
    pub ok: bool,
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevOrchestratorDump {
    pub tasks: Vec<DevTask>,
    pub reports: Vec<DevReport>,
    pub capability: DevCapability,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DevTaskStats {
    pub total: usize,
    pub queued: usize,
    pub planning: usize,
    pub executing: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub aborted: usize,
}

#[derive(Debug, Default)]
pub struct DevOrchestrator {
    tasks: RwLock<HashMap<String, DevTask>>,
    reports: RwLock<HashMap<String, DevReport>>,
    capability: RwLock<DevCapability>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevCapability {
    #[serde(default)]
    pub allow_execute: bool,
    pub allow_verify: bool,
    pub max_auto_iterations: u32,
    pub max_runtime_ms: u64,
}

impl Default for DevCapability {
    fn default() -> Self {
        Self {
            allow_execute: false,
            allow_verify: false,
            max_auto_iterations: 1,
            max_runtime_ms: 120_000,
        }
    }
}

impl DevOrchestrator {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub async fn set_capability(&self, capability: DevCapability) {
        *self.capability.write().await = capability;
    }

    pub async fn get_capability(&self) -> DevCapability {
        self.capability.read().await.clone()
    }

    pub async fn start_task(&self, prompt: String, verify_profile: VerifyProfile, repo_root: &Path) -> DevTask {
        let id = next_task_id();
        let mut task = DevTask {
            id: id.clone(),
            prompt,
            created_at_ms: now_ms(),
            status: DevTaskStatus::Queued,
            verify_profile,
            finished_at_ms: None,
        };
        self.tasks.write().await.insert(id.clone(), task.clone());

        self.update_status(&id, DevTaskStatus::Planning).await;
        self.update_status(&id, DevTaskStatus::Executing).await;

        let capability = self.get_capability().await;
        let run_result = self.run_task(&task, capability, repo_root).await;
        match run_result {
            Ok(report) => {
                let failed = report.error.is_some() || report.verify_ok == Some(false);
                self.reports.write().await.insert(id.clone(), report);
                self
                    .update_status(
                        &id,
                        if failed { DevTaskStatus::Failed } else { DevTaskStatus::Succeeded },
                    )
                    .await;
            }
            Err(err) => {
                let failed = DevReport {
                    task_id: id.clone(),
                    summary: "Phase A executor failed".to_string(),
                    changed_files: Vec::new(),
                    error: Some(err.to_string()),
                    workspace_dir: "".to_string(),
                    executor_command: None,
                    executor_ok: None,
                    executor_output: None,
                    git_diff: None,
                    verify_command: None,
                    verify_ok: None,
                    verify_output: None,
                    iteration_count: 0,
                    budget_exhausted: false,
                    replay_log: Vec::new(),
                    quality_score: 0,
                };
                self.reports.write().await.insert(id.clone(), failed);
                self.update_status(&id, DevTaskStatus::Failed).await;
            }
        }

        task = self.tasks.read().await.get(&id).cloned().unwrap_or(task);
        task
    }

    pub async fn rerun_task(&self, task_id: &str, repo_root: &Path) -> anyhow::Result<DevTask> {
        let task = self
            .tasks
            .read()
            .await
            .get(task_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;
        Ok(self.start_task(task.prompt, task.verify_profile, repo_root).await)
    }

    pub async fn abort_task(&self, task_id: &str) -> bool {
        if self.tasks.read().await.contains_key(task_id) {
            self.update_status(task_id, DevTaskStatus::Aborted).await;
            true
        } else {
            false
        }
    }

    pub async fn get_report(&self, task_id: &str) -> Option<DevReport> {
        self.reports.read().await.get(task_id).cloned()
    }

    pub async fn get_task(&self, task_id: &str) -> Option<DevTask> {
        self.tasks.read().await.get(task_id).cloned()
    }

    pub async fn task_snapshot(&self, task_id: &str) -> Option<DevTaskSnapshot> {
        let task = self.tasks.read().await.get(task_id).cloned()?;
        let report = self.reports.read().await.get(task_id).cloned();
        Some(DevTaskSnapshot { task, report })
    }

    pub async fn delete_task(&self, task_id: &str) -> bool {
        let existed_task = self.tasks.write().await.remove(task_id).is_some();
        self.reports.write().await.remove(task_id);
        existed_task
    }

    pub async fn delete_completed_tasks(&self) -> usize {
        let mut tasks = self.tasks.write().await;
        let completed_ids: Vec<String> = tasks
            .iter()
            .filter_map(|(id, t)| {
                if matches!(t.status, DevTaskStatus::Succeeded | DevTaskStatus::Failed | DevTaskStatus::Aborted) {
                    Some(id.clone())
                } else {
                    None
                }
            })
            .collect();

        for id in &completed_ids {
            tasks.remove(id);
        }
        drop(tasks);

        let mut reports = self.reports.write().await;
        for id in &completed_ids {
            reports.remove(id);
        }

        completed_ids.len()
    }

    pub async fn stats(&self) -> DevTaskStats {
        let tasks = self.tasks.read().await;
        let mut out = DevTaskStats::default();
        out.total = tasks.len();
        for t in tasks.values() {
            match t.status {
                DevTaskStatus::Queued => out.queued += 1,
                DevTaskStatus::Planning => out.planning += 1,
                DevTaskStatus::Executing => out.executing += 1,
                DevTaskStatus::Succeeded => out.succeeded += 1,
                DevTaskStatus::Failed => out.failed += 1,
                DevTaskStatus::Aborted => out.aborted += 1,
            }
        }
        out
    }

    pub async fn draft_pr_for_task(&self, task_id: &str) -> Option<DevPrDraft> {
        let task = self.get_task(task_id).await?;
        let report = self.get_report(task_id).await;

        let title = format!("self-dev: {}", task.prompt.chars().take(48).collect::<String>());
        let mut body = String::new();
        body.push_str("## Summary\n");
        body.push_str(&format!("- Task id: `{}`\n", task.id));
        body.push_str(&format!("- Prompt: {}\n", task.prompt));
        body.push_str(&format!("- Verify profile: `{:?}`\n", task.verify_profile));
        if let Some(r) = &report {
            body.push_str(&format!("- Workspace: `{}`\n", r.workspace_dir));
            body.push_str(&format!("- Changed files: {}\n", r.changed_files.len()));
            if let Some(cmd) = &r.verify_command {
                body.push_str(&format!("- Verify: `{}` => {:?}\n", cmd, r.verify_ok));
            }
            if let Some(cmd) = &r.executor_command {
                body.push_str(&format!("- Executor: `{}` => {:?}\n", cmd, r.executor_ok));
            }
            if r.git_diff.as_ref().is_some_and(|d| !d.trim().is_empty()) {
                body.push_str("- Diff: available in Self-Dev report\n");
            }
        }
        body.push_str("\n## Notes\n- Generated by Mori self-dev Phase B PR draft flow.\n");

        Some(DevPrDraft {
            task_id: task.id,
            title,
            body,
        })
    }

    pub async fn apply_reviewed_diff(
        &self,
        task_id: &str,
        repo_root: &Path,
    ) -> anyhow::Result<DevApplyResult> {
        let task = self
            .get_task(task_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("task not found: {task_id}"))?;
        if !matches!(task.status, DevTaskStatus::Succeeded) {
            anyhow::bail!("task is not succeeded: {task_id}");
        }

        let report = self
            .get_report(task_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("report not found: {task_id}"))?;
        if report.executor_ok != Some(true) {
            anyhow::bail!("executor did not complete successfully");
        }
        if report.error.is_some() {
            anyhow::bail!("report has error; review before applying");
        }
        if report.git_diff.as_ref().map_or(true, |d| d.trim().is_empty()) {
            anyhow::bail!("report has no reviewable diff");
        }

        let workspace_dir = PathBuf::from(&report.workspace_dir);
        let baseline_dir = workspace_dir.join("baseline");
        let repo_dir = workspace_dir.join("repo");
        if !baseline_dir.is_dir() || !repo_dir.is_dir() {
            anyhow::bail!("workspace baseline/repo missing for task {task_id}");
        }

        let changed_files = if report.changed_files.is_empty() {
            git_changed_files(&baseline_dir, &repo_dir).await?
        } else {
            report.changed_files.clone()
        };

        let mut applied_files = Vec::new();
        let mut deleted_files = Vec::new();
        for rel in changed_files {
            let safe_rel = safe_relative_path(&rel)?;
            let baseline_path = baseline_dir.join(&safe_rel);
            let source_path = repo_dir.join(&safe_rel);
            let target_path = repo_root.join(&safe_rel);

            if baseline_path.is_file() && target_path.exists() && !files_equal(&baseline_path, &target_path).await? {
                anyhow::bail!(
                    "refusing to overwrite changed file: {}",
                    safe_rel.display()
                );
            }
            if !baseline_path.exists() && target_path.exists() {
                anyhow::bail!(
                    "refusing to overwrite existing untracked target: {}",
                    safe_rel.display()
                );
            }

            if source_path.is_file() {
                copy_file(&source_path, &target_path).await?;
                applied_files.push(safe_rel.display().to_string());
            } else if baseline_path.is_file() {
                if target_path.exists() {
                    tokio::fs::remove_file(&target_path).await?;
                }
                deleted_files.push(safe_rel.display().to_string());
            } else {
                anyhow::bail!("changed file missing from workspace: {}", safe_rel.display());
            }
        }

        let result = DevApplyResult {
            task_id: task_id.to_string(),
            applied_files,
            deleted_files,
            command: format!(
                "apply reviewed diff from {} to {}",
                repo_dir.display(),
                repo_root.display()
            ),
            ok: true,
            output: "reviewed diff applied to working tree; no commit/push/merge performed".to_string(),
        };

        if let Some(report) = self.reports.write().await.get_mut(task_id) {
            report.replay_log.push(format!(
                "apply_reviewed_diff applied={} deleted={}",
                result.applied_files.len(),
                result.deleted_files.len()
            ));
        }

        Ok(result)
    }

    pub async fn export_dump(&self) -> DevOrchestratorDump {
        let tasks: Vec<DevTask> = self.tasks.read().await.values().cloned().collect();
        let reports: Vec<DevReport> = self.reports.read().await.values().cloned().collect();
        let capability = self.get_capability().await;
        DevOrchestratorDump { tasks, reports, capability }
    }

    pub async fn import_dump(&self, dump: DevOrchestratorDump) {
        let mut tasks_map = HashMap::new();
        for t in dump.tasks {
            tasks_map.insert(t.id.clone(), t);
        }
        let mut reports_map = HashMap::new();
        for r in dump.reports {
            reports_map.insert(r.task_id.clone(), r);
        }

        *self.tasks.write().await = tasks_map;
        *self.reports.write().await = reports_map;
        *self.capability.write().await = dump.capability;
    }

    pub async fn list_tasks(&self) -> Vec<DevTask> {
        let mut tasks: Vec<DevTask> = self.tasks.read().await.values().cloned().collect();
        tasks.sort_by_key(|t| t.created_at_ms);
        tasks.reverse();
        tasks
    }

    async fn update_status(&self, task_id: &str, status: DevTaskStatus) {
        if let Some(task) = self.tasks.write().await.get_mut(task_id) {
            task.status = status;
            if matches!(task.status, DevTaskStatus::Succeeded | DevTaskStatus::Failed | DevTaskStatus::Aborted) {
                task.finished_at_ms = Some(now_ms());
            }
        }
    }

    async fn run_task(
        &self,
        task: &DevTask,
        capability: DevCapability,
        repo_root: &Path,
    ) -> anyhow::Result<DevReport> {
        let workspace_dir = repo_root.join(".mori-dev-workspaces").join(&task.id);
        tokio::fs::create_dir_all(&workspace_dir).await?;

        let prompt_file = workspace_dir.join("PROMPT.md");
        let prompt_text = format!(
            "# DevTask {}\n\n## Prompt\n{}\n",
            task.id, task.prompt
        );
        tokio::fs::write(&prompt_file, prompt_text).await?;

        let started = now_ms();
        let mut iteration_count = 0u32;
        let mut budget_exhausted = false;
        let mut verify_command = None;
        let mut verify_ok = None;
        let mut verify_output = None;
        let mut replay_log: Vec<String> = vec![format!(
            "capability allow_execute={} allow_verify={} max_runtime_ms={}",
            capability.allow_execute, capability.allow_verify, capability.max_runtime_ms
        )];
        let mut executor_command = None;
        let mut executor_ok = None;
        let mut executor_output = None;
        let mut diff_text = None;
        let mut changed_files = vec![relativize(repo_root, &prompt_file)];
        let mut summary = "Created self-dev prompt; Codex executor not authorized".to_string();
        let mut error = None;
        let mut verify_root = repo_root.to_path_buf();
        let mut executor_workspace = None;

        if capability.allow_execute {
            let worktree_dir = workspace_dir.join("repo");
            let baseline_dir = workspace_dir.join("baseline");
            replay_log.push(format!("creating isolated workspace at {}", worktree_dir.display()));
            let workspace = create_workspace_snapshot(repo_root, &baseline_dir, &worktree_dir).await;
            match workspace {
                Ok(file_count) => {
                    replay_log.push(format!("workspace snapshot copied {} tracked file(s)", file_count));
                    verify_root = worktree_dir.clone();
                    executor_workspace = Some(worktree_dir.clone());

                    let codex_prompt = build_codex_prompt(task);
                    tokio::fs::write(workspace_dir.join("CODEX_PROMPT.md"), &codex_prompt).await?;
                    let timeout_ms = capability.max_runtime_ms.max(30_000);
                    let codex = run_codex_executor(&worktree_dir, &codex_prompt, timeout_ms).await;
                    executor_command = Some(codex.command);
                    executor_ok = Some(codex.ok);
                    tokio::fs::write(workspace_dir.join("CODEX_OUTPUT.txt"), &codex.output).await?;
                    replay_log.push(format!("codex executor ok={}", codex.ok));
                    executor_output = Some(codex.output);

                    clean_generated_paths(&worktree_dir).await;
                    let diff = git_diff(&baseline_dir, &worktree_dir).await.unwrap_or_else(|e| {
                        format!("failed to collect git diff: {e}")
                    });
                    tokio::fs::write(workspace_dir.join("DIFF.patch"), &diff).await?;
                    let files = git_changed_files(&baseline_dir, &worktree_dir).await.unwrap_or_default();
                    changed_files = files;
                    diff_text = Some(diff);

                    if !codex.ok {
                        error = Some("Codex executor failed; see executor output".to_string());
                        summary = "Codex executor failed before producing a ready diff".to_string();
                    } else if changed_files.is_empty() {
                        error = Some("Codex completed but produced no git diff".to_string());
                        summary = "Codex completed without file changes".to_string();
                    } else {
                        summary = format!(
                            "Codex executor completed with {} changed file(s)",
                            changed_files.len()
                        );
                    }
                }
                Err(e) => {
                    error = Some(format!("failed to create isolated workspace: {e}"));
                    summary = "Codex executor could not start because workspace setup failed".to_string();
                }
            }
        } else {
            let plan_file = workspace_dir.join("PLAN.md");
            let plan = format!(
                "# DevTask {}\n\n## Prompt\n{}\n\n## Note\nPhase B executor is disabled until a human enables Codex execution. No commit/push/merge performed.\n",
                task.id, task.prompt
            );
            tokio::fs::write(&plan_file, plan).await?;
            changed_files.push(relativize(repo_root, &plan_file));
        }

        if capability.allow_verify {
            let rounds = capability.max_auto_iterations.max(1);
            for _ in 0..rounds {
                iteration_count += 1;
                let (cmd, ok, out) = run_verify_profile(task.verify_profile, &verify_root).await;
                replay_log.push(format!("iteration {} => verify_ok={:?}", iteration_count, ok));
                verify_command = cmd;
                verify_ok = ok;
                verify_output = out;

                let elapsed = (now_ms() - started).max(0) as u64;
                if elapsed >= capability.max_runtime_ms {
                    budget_exhausted = true;
                    break;
                }
                if verify_ok == Some(true) {
                    break;
                }
            }
            if let Some(workspace) = executor_workspace.as_deref() {
                clean_generated_paths(workspace).await;
            }
        }

        let quality_score = compute_quality_score(verify_ok, budget_exhausted, iteration_count);

        Ok(DevReport {
            task_id: task.id.clone(),
            summary,
            changed_files,
            error,
            workspace_dir: workspace_dir.display().to_string(),
            executor_command,
            executor_ok,
            executor_output,
            git_diff: diff_text,
            verify_command,
            verify_ok,
            verify_output,
            iteration_count,
            budget_exhausted,
            replay_log,
            quality_score,
        })
    }
}

#[derive(Debug)]
struct CommandOutcome {
    command: String,
    ok: bool,
    output: String,
}

fn build_codex_prompt(task: &DevTask) -> String {
    format!(
        "You are running inside a disposable workspace copy for Mori Desktop Self-Dev Phase B.\n\
         Implement the requested change in this workspace copy only.\n\n\
         Hard rules:\n\
         - Do not commit, push, merge, tag, or open a PR.\n\
         - Do not edit files outside this workspace.\n\
         - Keep the change narrow and consistent with the repo.\n\
         - Do not run sudo or install system packages.\n\
         - Leave a reviewable git diff; Mori will collect `git diff` and run the selected verifier after you exit.\n\n\
         User task:\n{}\n",
        task.prompt
    )
}

async fn create_workspace_snapshot(
    repo_root: &Path,
    baseline_dir: &Path,
    worktree_dir: &Path,
) -> anyhow::Result<usize> {
    tokio::fs::create_dir_all(baseline_dir).await?;
    tokio::fs::create_dir_all(worktree_dir).await?;

    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo_root)
        .arg("ls-files")
        .arg("-z");
    let out = run_output(cmd, 120_000).await?;
    if !out.ok {
        return Err(anyhow::anyhow!(out.output));
    }

    let mut copied = 0usize;
    for rel in out.output.split('\0').filter(|p| !p.is_empty()) {
        let source = repo_root.join(rel);
        if !source.is_file() {
            continue;
        }
        copy_file(&source, &baseline_dir.join(rel)).await?;
        copy_file(&source, &worktree_dir.join(rel)).await?;
        copied += 1;
    }
    Ok(copied)
}

async fn copy_file(source: &Path, target: &Path) -> anyhow::Result<()> {
    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::copy(source, target).await?;
    Ok(())
}

async fn run_codex_executor(workspace_dir: &Path, prompt: &str, timeout_ms: u64) -> CommandOutcome {
    let codex_bin = std::env::var("MORI_SELF_DEV_CODEX_BIN").unwrap_or_else(|_| "codex".to_string());
    let mut cmd = Command::new(&codex_bin);
    cmd.arg("--sandbox")
        .arg("workspace-write")
        .arg("--ask-for-approval")
        .arg("never")
        .arg("exec")
        .arg("--ephemeral")
        .arg("--skip-git-repo-check")
        .arg("--color")
        .arg("never")
        .arg("-C")
        .arg(workspace_dir)
        .arg("-");
    if let Some(ceiling) = workspace_dir.parent() {
        cmd.env("GIT_CEILING_DIRECTORIES", ceiling);
    }
    let command = format!(
        "{} --sandbox workspace-write --ask-for-approval never exec --ephemeral --skip-git-repo-check --color never -C {} -",
        codex_bin,
        workspace_dir.display()
    );
    run_output_with_stdin(cmd, command, prompt.as_bytes(), timeout_ms).await
}

async fn clean_generated_paths(workspace_dir: &Path) {
    for rel in [
        "target",
        "dist",
        ".vite",
        "node_modules",
        "tsconfig.tsbuildinfo",
        "crates/mori-tauri/target",
        "crates/mori-tauri/gen",
        ".mori-dev-workspaces",
        ".git",
    ] {
        let path = workspace_dir.join(rel);
        let _ = tokio::fs::remove_file(&path).await;
        let _ = tokio::fs::remove_dir_all(&path).await;
    }
}

async fn git_diff(baseline_dir: &Path, worktree_dir: &Path) -> anyhow::Result<String> {
    let mut cmd = Command::new("git");
    cmd.arg("diff")
        .arg("--no-index")
        .arg("--binary")
        .arg(baseline_dir)
        .arg(".");
    cmd.current_dir(worktree_dir);
    let out = run_output(cmd, 120_000).await?;
    if out.ok || !out.output.trim().is_empty() {
        Ok(out.output)
    } else {
        Err(anyhow::anyhow!(out.output))
    }
}

async fn git_changed_files(baseline_dir: &Path, worktree_dir: &Path) -> anyhow::Result<Vec<String>> {
    let mut cmd = Command::new("git");
    cmd.arg("diff")
        .arg("--no-index")
        .arg("--name-only")
        .arg(baseline_dir)
        .arg(".");
    cmd.current_dir(worktree_dir);
    let out = run_output(cmd, 120_000).await?;
    if !out.ok && out.output.trim().is_empty() {
        return Err(anyhow::anyhow!(out.output));
    }
    let mut files = Vec::new();
    Ok(out
        .output
        .lines()
        .filter_map(|line| normalize_diff_name(line, baseline_dir, worktree_dir))
        .filter(|line| {
            if files.contains(line) {
                false
            } else {
                files.push(line.clone());
                true
            }
        })
        .collect())
}

fn normalize_diff_name(line: &str, baseline_dir: &Path, worktree_dir: &Path) -> Option<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let path = Path::new(trimmed);
    if let Ok(rel) = path.strip_prefix(worktree_dir) {
        return Some(rel.display().to_string());
    }
    if let Ok(rel) = path.strip_prefix(baseline_dir) {
        return Some(rel.display().to_string());
    }
    Some(trimmed.trim_start_matches("./").to_string())
}

fn safe_relative_path(rel: &str) -> anyhow::Result<PathBuf> {
    if rel.contains('\0') {
        anyhow::bail!("path contains NUL byte");
    }

    let path = Path::new(rel);
    if path.is_absolute() {
        anyhow::bail!("absolute paths are not allowed: {rel}");
    }

    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => safe.push(part),
            std::path::Component::CurDir => {}
            _ => anyhow::bail!("unsafe path component in {rel}"),
        }
    }

    if safe.as_os_str().is_empty() {
        anyhow::bail!("empty path");
    }

    Ok(safe)
}

async fn files_equal(a: &Path, b: &Path) -> anyhow::Result<bool> {
    let a_meta = tokio::fs::metadata(a).await?;
    let b_meta = tokio::fs::metadata(b).await?;
    if a_meta.len() != b_meta.len() {
        return Ok(false);
    }
    let a_bytes = tokio::fs::read(a).await?;
    let b_bytes = tokio::fs::read(b).await?;
    Ok(a_bytes == b_bytes)
}

async fn run_output(cmd: Command, timeout_ms: u64) -> anyhow::Result<CommandOutcome> {
    let command = format!("{cmd:?}");
    Ok(run_output_with_stdin(cmd, command, &[], timeout_ms).await)
}

async fn run_output_with_stdin(
    mut cmd: Command,
    command: String,
    stdin_bytes: &[u8],
    timeout_ms: u64,
) -> CommandOutcome {
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let spawn = cmd.spawn();
    let mut child = match spawn {
        Ok(child) => child,
        Err(e) => {
            return CommandOutcome {
                command,
                ok: false,
                output: format!("spawn failed: {e}"),
            };
        }
    };

    if !stdin_bytes.is_empty() {
        if let Some(mut stdin) = child.stdin.take() {
            if let Err(e) = stdin.write_all(stdin_bytes).await {
                return CommandOutcome {
                    command,
                    ok: false,
                    output: format!("write stdin failed: {e}"),
                };
            }
        }
    }

    let waited = tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        child.wait_with_output(),
    )
    .await;

    match waited {
        Ok(Ok(out)) => CommandOutcome {
            command,
            ok: out.status.success(),
            output: merge_output(&out.stdout, &out.stderr),
        },
        Ok(Err(e)) => CommandOutcome {
            command,
            ok: false,
            output: format!("wait failed: {e}"),
        },
        Err(_) => CommandOutcome {
            command,
            ok: false,
            output: format!("command timed out after {timeout_ms}ms"),
        },
    }
}

fn merge_output(stdout: &[u8], stderr: &[u8]) -> String {
    let mut merged = String::new();
    merged.push_str(&String::from_utf8_lossy(stdout));
    merged.push_str(&String::from_utf8_lossy(stderr));
    merged
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

static NEXT_TASK_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn next_task_id() -> String {
    let seq = NEXT_TASK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("dev-{}-{seq}", now_ms())
}

fn relativize(root: &Path, target: &PathBuf) -> String {
    target
        .strip_prefix(root)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| target.display().to_string())
}

fn compute_quality_score(verify_ok: Option<bool>, budget_exhausted: bool, iterations: u32) -> u8 {
    let mut score: i32 = if verify_ok == Some(true) { 85 } else { 45 };
    if budget_exhausted { score -= 25; }
    score -= (iterations.saturating_sub(1) as i32) * 5;
    score.clamp(0, 100) as u8
}

async fn run_verify_profile(
    profile: VerifyProfile,
    repo_root: &Path,
) -> (Option<String>, Option<bool>, Option<String>) {
    let command = match profile {
        VerifyProfile::None => return (None, None, None),
        VerifyProfile::Quick => "cargo check -p mori-core",
        VerifyProfile::Full => "bash scripts/verify.sh",
    };

    let output = Command::new("bash")
        .arg("-lc")
        .arg(command)
        .current_dir(repo_root)
        .output()
        .await;

    match output {
        Ok(out) => {
            let merged = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            (Some(command.to_string()), Some(out.status.success()), Some(merged))
        }
        Err(err) => (
            Some(command.to_string()),
            Some(false),
            Some(format!("failed to run verify command: {err}")),
        ),
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn snapshot_includes_task_and_report_after_start() {
        let root = tempfile::tempdir().unwrap();
        let orch = DevOrchestrator::new();
        let task = orch
            .start_task("hello".to_string(), VerifyProfile::None, root.path())
            .await;

        let snap = orch.task_snapshot(&task.id).await.expect("snapshot exists");
        assert_eq!(snap.task.id, task.id);
        assert!(snap.report.is_some());
        assert_eq!(snap.report.unwrap().task_id, task.id);
    }

    #[tokio::test]
    async fn rerun_creates_new_task_id() {
        let root = tempfile::tempdir().unwrap();
        let orch = DevOrchestrator::new();
        let first = orch
            .start_task("hello".to_string(), VerifyProfile::None, root.path())
            .await;
        let second = orch.rerun_task(&first.id, root.path()).await.unwrap();

        assert_ne!(first.id, second.id);
        assert_eq!(second.prompt, first.prompt);
        assert!(orch.task_snapshot(&second.id).await.is_some());
    }

    #[tokio::test]
    async fn delete_task_removes_task_and_report() {
        let root = tempfile::tempdir().unwrap();
        let orch = DevOrchestrator::new();
        let task = orch
            .start_task("delete me".to_string(), VerifyProfile::None, root.path())
            .await;

        assert!(orch.task_snapshot(&task.id).await.is_some());
        assert!(orch.delete_task(&task.id).await);
        assert!(orch.task_snapshot(&task.id).await.is_none());
        assert!(!orch.delete_task(&task.id).await);
    }

    #[tokio::test]
    async fn delete_completed_tasks_keeps_running() {
        let root = tempfile::tempdir().unwrap();
        let orch = DevOrchestrator::new();

        let done = orch
            .start_task("done".to_string(), VerifyProfile::None, root.path())
            .await;

        let running_id = "running-1".to_string();
        orch.tasks.write().await.insert(
            running_id.clone(),
            DevTask {
                id: running_id.clone(),
                prompt: "still running".to_string(),
                created_at_ms: 1,
                status: DevTaskStatus::Executing,
                verify_profile: VerifyProfile::None,
                finished_at_ms: None,
            },
        );

        let removed = orch.delete_completed_tasks().await;
        assert_eq!(removed, 1);
        assert!(orch.task_snapshot(&done.id).await.is_none());
        assert!(orch.get_task(&running_id).await.is_some());
    }

    #[tokio::test]
    async fn stats_counts_match_task_states() {
        let root = tempfile::tempdir().unwrap();
        let orch = DevOrchestrator::new();

        let _ok = orch
            .start_task("ok".to_string(), VerifyProfile::None, root.path())
            .await;

        orch.tasks.write().await.insert(
            "q".to_string(),
            DevTask {
                id: "q".to_string(),
                prompt: "q".to_string(),
                created_at_ms: 1,
                status: DevTaskStatus::Queued,
                verify_profile: VerifyProfile::None,
                finished_at_ms: None,
            },
        );

        let stats = orch.stats().await;
        assert!(stats.total >= 2);
        assert!(stats.succeeded >= 1);
        assert!(stats.queued >= 1);
    }

    #[tokio::test]
    async fn quality_score_is_bounded() {
        assert!(compute_quality_score(Some(true), false, 1) <= 100);
        assert!(compute_quality_score(Some(false), true, 9) <= 100);
    }

    #[tokio::test]
    async fn phase_c_budget_defaults() {
        let c = DevCapability::default();
        assert!(!c.allow_execute);
        assert_eq!(c.max_auto_iterations, 1);
        assert_eq!(c.max_runtime_ms, 120_000);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn phase_b_executor_creates_diff_report() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().unwrap();
        let fake_codex = root.path().join("fake-codex");
        tokio::fs::write(
            &fake_codex,
            r#"#!/usr/bin/env bash
set -euo pipefail
workspace=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "-C" ]; then
    workspace="$2"
    shift 2
    continue
  fi
  shift
done
if [ -z "$workspace" ]; then
  echo "missing workspace" >&2
  exit 2
fi
cat >/dev/null
printf 'after\n' > "$workspace/hello.txt"
echo "fake codex wrote hello.txt"
"#,
        )
        .await
        .unwrap();
        let mut permissions = std::fs::metadata(&fake_codex).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&fake_codex, permissions).unwrap();

        std::fs::write(root.path().join("hello.txt"), "before\n").unwrap();
        assert!(std::process::Command::new("git")
            .arg("init")
            .current_dir(root.path())
            .status()
            .unwrap()
            .success());
        assert!(std::process::Command::new("git")
            .arg("add")
            .arg("hello.txt")
            .current_dir(root.path())
            .status()
            .unwrap()
            .success());

        std::env::set_var("MORI_SELF_DEV_CODEX_BIN", &fake_codex);
        let orch = DevOrchestrator::new();
        orch.set_capability(DevCapability {
            allow_execute: true,
            allow_verify: false,
            max_auto_iterations: 1,
            max_runtime_ms: 30_000,
        }).await;
        let task = orch
            .start_task("change hello".to_string(), VerifyProfile::None, root.path())
            .await;
        std::env::remove_var("MORI_SELF_DEV_CODEX_BIN");

        let snap = orch.task_snapshot(&task.id).await.expect("snapshot exists");
        assert!(matches!(snap.task.status, DevTaskStatus::Succeeded));
        let report = snap.report.expect("report exists");
        assert_eq!(report.executor_ok, Some(true));
        assert!(report.executor_output.unwrap().contains("fake codex wrote hello.txt"));
        assert!(report.changed_files.iter().any(|f| f.ends_with("hello.txt")));
        assert!(report.git_diff.unwrap().contains("-before"));
        assert!(report.error.is_none());
        assert_eq!(std::fs::read_to_string(root.path().join("hello.txt")).unwrap(), "before\n");

        let applied = orch.apply_reviewed_diff(&task.id, root.path()).await.unwrap();
        assert!(applied.ok);
        assert_eq!(applied.applied_files, vec!["hello.txt"]);
        assert!(applied.deleted_files.is_empty());
        assert_eq!(std::fs::read_to_string(root.path().join("hello.txt")).unwrap(), "after\n");
    }


    #[tokio::test]
    async fn e2_regression_flow_lifecycle_smoke() {
        let root = tempfile::tempdir().unwrap();
        let orch = DevOrchestrator::new();

        let first = orch
            .start_task("e2 smoke".to_string(), VerifyProfile::None, root.path())
            .await;

        let tasks = orch.list_tasks().await;
        assert!(tasks.iter().any(|t| t.id == first.id));

        let snap = orch.task_snapshot(&first.id).await.expect("snapshot exists");
        assert!(snap.report.is_some());

        let rerun = orch.rerun_task(&first.id, root.path()).await.unwrap();
        assert_ne!(first.id, rerun.id);

        let aborted = orch.abort_task(&rerun.id).await;
        assert!(aborted);

        let deleted = orch.delete_task(&first.id).await;
        assert!(deleted);
    }

    #[tokio::test]
    async fn export_import_dump_roundtrip() {
        let root = tempfile::tempdir().unwrap();
        let orch = DevOrchestrator::new();
        orch.set_capability(DevCapability {
            allow_execute: false,
            allow_verify: true,
            max_auto_iterations: 2,
            max_runtime_ms: 120_000,
        }).await;
        let task = orch
            .start_task("persist".to_string(), VerifyProfile::None, root.path())
            .await;

        let dump = orch.export_dump().await;

        let orch2 = DevOrchestrator::new();
        orch2.import_dump(dump).await;

        assert!(orch2.get_task(&task.id).await.is_some());
        assert!(orch2.get_report(&task.id).await.is_some());
        assert!(orch2.get_capability().await.allow_verify);
    }
}
