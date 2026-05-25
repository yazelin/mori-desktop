use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
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
    pub allow_verify: bool,
    pub max_auto_iterations: u32,
    pub max_runtime_ms: u64,
}

impl Default for DevCapability {
    fn default() -> Self {
        Self { allow_verify: false, max_auto_iterations: 1, max_runtime_ms: 120_000 }
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
        let id = format!("dev-{}", now_ms());
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
                self.reports.write().await.insert(id.clone(), report);
                self.update_status(&id, DevTaskStatus::Succeeded).await;
            }
            Err(err) => {
                let failed = DevReport {
                    task_id: id.clone(),
                    summary: "Phase A executor failed".to_string(),
                    changed_files: Vec::new(),
                    error: Some(err.to_string()),
                    workspace_dir: "".to_string(),
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
        }
        body.push_str("\n## Notes\n- Generated by Mori self-dev Phase B PR draft flow.\n");

        Some(DevPrDraft {
            task_id: task.id,
            title,
            body,
        })
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

        let plan_file = workspace_dir.join("PLAN.md");
        let plan = format!(
            "# DevTask {}\n\n## Prompt\n{}\n\n## Note\nPhase A skeleton only; no commit/push performed.\n",
            task.id, task.prompt
        );
        tokio::fs::write(&plan_file, plan).await?;

        let started = now_ms();
        let mut iteration_count = 0u32;
        let mut budget_exhausted = false;
        let mut verify_command = None;
        let mut verify_ok = None;
        let mut verify_output = None;
        let mut replay_log: Vec<String> = Vec::new();

        if capability.allow_verify {
            let rounds = capability.max_auto_iterations.max(1);
            for _ in 0..rounds {
                iteration_count += 1;
                let (cmd, ok, out) = run_verify_profile(task.verify_profile, repo_root).await;
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
        }

        let quality_score = compute_quality_score(verify_ok, budget_exhausted, iteration_count);

        Ok(DevReport {
            task_id: task.id.clone(),
            summary: "Created isolated workspace and scaffold plan".to_string(),
            changed_files: vec![relativize(repo_root, &plan_file)],
            error: None,
            workspace_dir: workspace_dir.display().to_string(),
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

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
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
        assert_eq!(c.max_auto_iterations, 1);
        assert_eq!(c.max_runtime_ms, 120_000);
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
        orch.set_capability(DevCapability { allow_verify: true, max_auto_iterations: 2, max_runtime_ms: 120_000 }).await;
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
