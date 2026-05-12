//! LLM 通訊抽象。
//!
//! 一份 agent 邏輯能打 Groq / Ollama / OpenAI / Anthropic 等任意 OpenAI 相容後端。
//! 每個 Skill 可指定偏好的 provider + model,允許:
//! - 任務 → 模型精細搭配
//! - Fallback chain
//! - Privacy::LocalOnly 強制本地
//!
//! 訊息結構支援 OpenAI tool-calling 多輪協定:
//! - `system` / `user`:role + content
//! - `assistant`(發起 tool_call):role + tool_calls(content 可能也有)
//! - `tool`(回傳結果):role + content + tool_call_id + name

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod bash_cli_agent;
pub mod claude_cli;
pub mod generic_openai;
pub mod groq;
pub mod ollama;
pub(crate) mod openai_compat;
pub mod transcribe;
pub mod whisper_local;

// ─── Provider factory ───────────────────────────────────────────────
//
// `build_chat_provider` 讀 `~/.mori/config.json` 的 `provider`
// 欄位,構造對應 LlmProvider 回傳。Groq / Ollama 走不同 default。
// retry_callback 只對 Groq 有意義(Ollama 本機沒 rate limit)。

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// 由名字蓋一個 LlmProvider。`name` 是 config 裡的 provider key
/// ("groq" / "ollama" / "claude-cli")。retry_cb 只在 Groq 用上,其他 ignore。
///
/// 不知道的 name 會回 Err,讓呼叫端能看到錯字 — 不像 `build_chat_provider`
/// 把未知值 silently fallback 到 groq。Routing 路徑要嚴一點。
pub fn build_named_provider(
    name: &str,
    retry_cb: Option<groq::RetryCallback>,
) -> anyhow::Result<Arc<dyn LlmProvider>> {
    match name {
        "ollama" => {
            let base_url = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/ollama/base_url"))
                .unwrap_or_else(|| ollama::OllamaProvider::DEFAULT_BASE_URL.to_string());
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/ollama/model"))
                .unwrap_or_else(|| ollama::OllamaProvider::DEFAULT_MODEL.to_string());
            Ok(Arc::new(ollama::OllamaProvider::new(base_url, model)))
        }
        "claude-cli" => {
            let binary = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/claude-cli/binary"))
                .unwrap_or_else(|| claude_cli::ClaudeCliProvider::DEFAULT_BINARY.to_string());
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/claude-cli/model"));
            Ok(Arc::new(claude_cli::ClaudeCliProvider::new(binary, model)))
        }
        "claude-bash" => {
            // 5D:Bash CLI proxy。claude (或將來 codex/gemini) 走它們自己的
            // 內部 reasoning,透過 Bash 工具呼叫 mori CLI dispatch skill。
            let binary = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/claude-bash/binary"))
                .unwrap_or_else(|| {
                    bash_cli_agent::BashCliAgentProvider::DEFAULT_BINARY.to_string()
                });
            let mori_cli = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/claude-bash/mori_cli_path"))
                .map(std::path::PathBuf::from)
                .unwrap_or_else(bash_cli_agent::BashCliAgentProvider::detect_mori_cli);
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/claude-bash/model"));
            Ok(Arc::new(bash_cli_agent::BashCliAgentProvider::new(
                binary, mori_cli, model,
            )))
        }
        "gemini-bash" => {
            let binary = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/gemini-bash/binary"))
                .unwrap_or_else(|| "gemini".to_string());
            let mori_cli = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/gemini-bash/mori_cli_path"))
                .map(std::path::PathBuf::from)
                .unwrap_or_else(bash_cli_agent::BashCliAgentProvider::detect_mori_cli);
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/gemini-bash/model"));
            Ok(Arc::new(bash_cli_agent::BashCliAgentProvider::new(binary, mori_cli, model)))
        }
        "codex-bash" => {
            let binary = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/codex-bash/binary"))
                .unwrap_or_else(|| "codex".to_string());
            let mori_cli = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/codex-bash/mori_cli_path"))
                .map(std::path::PathBuf::from)
                .unwrap_or_else(bash_cli_agent::BashCliAgentProvider::detect_mori_cli);
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/codex-bash/model"));
            Ok(Arc::new(bash_cli_agent::BashCliAgentProvider::new(binary, mori_cli, model)))
        }
        // 5D-3:chat-only 變體 — 類似 claude-cli,但走 gemini / codex binary。
        // 省略 agent 旗標(--yolo / --dangerously-bypass-approvals-and-sandbox)→
        // non-TTY 下 tool 執行無法被核准 → 純文字 in/out,可用於 routing.skills。
        "gemini-cli" => {
            let binary = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/gemini-cli/binary"))
                .unwrap_or_else(|| "gemini".to_string());
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/gemini-cli/model"));
            Ok(Arc::new(bash_cli_agent::BashCliAgentProvider::new_with_protocol(
                binary,
                std::path::PathBuf::from("mori"),
                model,
                bash_cli_agent::CliProtocol::GeminiChat,
            )))
        }
        "codex-cli" => {
            let binary = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/codex-cli/binary"))
                .unwrap_or_else(|| "codex".to_string());
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/codex-cli/model"));
            Ok(Arc::new(bash_cli_agent::BashCliAgentProvider::new_with_protocol(
                binary,
                std::path::PathBuf::from("mori"),
                model,
                bash_cli_agent::CliProtocol::CodexChat,
            )))
        }
        "groq" => {
            let key = groq::GroqProvider::discover_api_key().ok_or_else(|| {
                anyhow::anyhow!(
                    "no GROQ_API_KEY configured. Edit ~/.mori/config.json or set $GROQ_API_KEY"
                )
            })?;
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/groq/model"))
                .unwrap_or_else(|| groq::GroqProvider::DEFAULT_MODEL.to_string());
            let p = groq::GroqProvider::new(key, model);
            let p = if let Some(cb) = retry_cb {
                p.with_retry_callback(cb)
            } else {
                p
            };
            Ok(Arc::new(p))
        }
        // 5J-1: gemini = OpenAI-compatible 包裝。Google 的 generativelanguage
        // endpoint 接 OpenAI Chat Completions wire format。key 從 GEMINI_API_KEY
        // env 或 ~/.mori/config.json api_keys.GEMINI_API_KEY 取。
        "gemini" => {
            let api_key = resolve_api_key("GEMINI_API_KEY").ok_or_else(|| {
                anyhow::anyhow!(
                    "no GEMINI_API_KEY configured. Edit ~/.mori/config.json \
                     api_keys.GEMINI_API_KEY or set $GEMINI_API_KEY"
                )
            })?;
            let api_base = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/gemini/api_base"))
                .unwrap_or_else(|| GEMINI_DEFAULT_API_BASE.to_string());
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/gemini/model"))
                .unwrap_or_else(|| GEMINI_DEFAULT_MODEL.to_string());
            Ok(Arc::new(
                generic_openai::GenericOpenAiProvider::new(api_base, api_key, model)
                    .with_name("gemini"),
            ))
        }
        other => {
            // 5N: 自訂 OpenAI-compat 端點 — 在 config.json `providers.<other>` 設:
            //   {
            //     "api_base": "https://your.openai.azure.com/openai/v1",
            //     "api_key_env": "ZEROTYPE_AIPROMPT_API_KEY_AOAI",  // 或 api_key 直接寫(不建議)
            //     "model": "gpt-4.1"
            //   }
            // 有 api_base 就視為 OpenAI-compat,沒 api_base 就 bail unknown。
            //
            // 取代過往 ZEROTYPE_AIPROMPT_* frontmatter 鍵的角色 — 端點細節集中在
            // config.json,profile 只 `provider: <name>`。
            let cfg_path = mori_config_path();
            let cfg = cfg_path.as_deref();
            let api_base = cfg
                .and_then(|p| groq::read_json_pointer(p, &format!("/providers/{other}/api_base")));
            if let Some(api_base) = api_base {
                let key_env = cfg.and_then(|p| {
                    groq::read_json_pointer(p, &format!("/providers/{other}/api_key_env"))
                });
                let inline_key = cfg.and_then(|p| {
                    groq::read_json_pointer(p, &format!("/providers/{other}/api_key"))
                });
                let api_key = match (key_env.as_deref(), inline_key.as_deref()) {
                    (Some(env_name), _) => resolve_api_key(env_name).unwrap_or_default(),
                    (None, Some(k)) => k.to_string(),
                    (None, None) => String::new(),
                };
                if api_key.is_empty() {
                    anyhow::bail!(
                        "custom provider '{}' has api_base but api_key 解不到。請在 \
                         ~/.mori/config.json 設 providers.{}.api_key_env(指向 OS env \
                         或 api_keys.<name>;建議),或 .api_key(直接寫死,不建議)。",
                        other,
                        other
                    );
                }
                let model = cfg
                    .and_then(|p| groq::read_json_pointer(p, &format!("/providers/{other}/model")))
                    .unwrap_or_default();
                if model.is_empty() {
                    tracing::warn!(
                        provider = other,
                        "custom provider 沒設 providers.{}.model — 空 model 大概會被 API 拒絕",
                        other
                    );
                }
                tracing::info!(
                    provider = other,
                    api_base = %api_base,
                    model = %model,
                    "custom OpenAI-compat provider built from config.json"
                );
                return Ok(Arc::new(
                    generic_openai::GenericOpenAiProvider::new(api_base, api_key, model)
                        .with_name_owned(other.to_string()),
                ));
            }
            anyhow::bail!(
                "unknown provider '{}' — built-in: groq, gemini, ollama, claude-cli, \
                 claude-bash, gemini-bash, codex-bash, gemini-cli, codex-cli。要用自訂 \
                 OpenAI-compat 端點請在 ~/.mori/config.json 設 providers.{}.api_base + \
                 .api_key_env + .model。",
                other,
                other
            )
        }
    }
}

// Gemini defaults — 拉出來給 build_named_provider + active_chat_provider_snapshot 共用。
pub(crate) const GEMINI_DEFAULT_API_BASE: &str =
    "https://generativelanguage.googleapis.com/v1beta/openai/";
pub(crate) const GEMINI_DEFAULT_MODEL: &str = "gemini-3.1-flash-lite-preview";

/// 從 OS env var 或 ~/.mori/config.json `api_keys.<name>` 取 API key。
/// env var 優先；空字串視為未設。
pub(crate) fn resolve_api_key(key_env_name: &str) -> Option<String> {
    resolve_api_key_at(mori_config_path().as_deref(), key_env_name)
}

/// 純函式版本（可測）：給定 config path（可能為 None）+ key 環境變數名。
pub(crate) fn resolve_api_key_at(config_path: Option<&std::path::Path>, key_env_name: &str) -> Option<String> {
    if let Ok(v) = std::env::var(key_env_name) {
        if !v.is_empty() {
            return Some(v);
        }
    }
    config_path
        .and_then(|p| groq::read_json_pointer(p, &format!("/api_keys/{key_env_name}")))
        .filter(|s| !s.is_empty())
}

/// ZeroType `ZEROTYPE_AIPROMPT_*` 三個 frontmatter 鍵 → openai-compatible 臨時 provider。
pub fn build_openai_compat_provider(
    api_base: impl Into<String>,
    api_key: impl Into<String>,
    model: impl Into<String>,
) -> Arc<dyn LlmProvider> {
    Arc::new(generic_openai::GenericOpenAiProvider::new(api_base, api_key, model))
}

/// 從 `~/.mori/config.json` 蓋出**主 chat provider**。配置:
/// - `provider`: "groq"(預設) | "ollama" | "claude-cli"
/// - `providers.<name>.<...>` 各 provider 細節
///
/// 未知 provider 會 silently fallback 到 groq + warn(舊行為,
/// 不破壞既有 user)。retry_callback 只在 Groq 路徑套用。
///
/// **Note**:5A-3 起若有 `routing` 區塊,主 agent 應該用 [`Routing`]
/// 而不是這個函式;這個只給沒設 routing 的舊路徑用。
pub fn build_chat_provider(
    retry_cb: Option<groq::RetryCallback>,
) -> anyhow::Result<Arc<dyn LlmProvider>> {
    let default = read_provider_config();
    // 5N: 不再 allowlist — 直接交給 build_named_provider 決定;5 個 built-in 之外
    // 會走 config.json `providers.<name>` lookup(custom OpenAI-compat)。失敗才
    // fallback 到 groq + warn,讓 user 看到自訂端點哪裡沒填齊。
    let p = match build_named_provider(&default, retry_cb.clone()) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                provider = %default,
                error = %e,
                "could not build configured provider — falling back to 'groq'",
            );
            build_named_provider("groq", retry_cb)?
        }
    };
    tracing::info!(provider = %p.name(), model = %p.model(), "chat provider selected");
    Ok(p)
}

/// 5A-3:per-skill provider routing。Mori agent loop 跟每個 skill 內部
/// chat 都可以指定不同 provider,用來:
/// - 讓 agent 走 Groq tool-calling(快、會 dispatch tool),skill 內部
///   推理走 Claude CLI(quota 用 user 自己的 Pro/Max)或 Ollama(本機免錢)
/// - 之後加 fallback chain(5A-3b)時也以這個結構為基礎
///
/// 沒有 `routing` block 的 config 會退化成全部用 `provider`,
/// 跟 5A-2 之前的行為一致。
pub struct Routing {
    /// 主 agent loop 的 provider。**必須** supports_tool_calling,否則
    /// skill dispatch 會失效(只會拿到純文字 fallback)— build 時若
    /// 不支援會 warn 但不 fail。
    pub agent: Arc<dyn LlmProvider>,
    /// Skill 名字 → provider 的 override map。Map 沒列到的 skill 用
    /// `skill_fallback`(通常 = agent,但 agent 是 agent-only 型(如
    /// bash-cli-agent)時會自動切到 claude-cli 防遞迴)。
    pub skills: HashMap<String, Arc<dyn LlmProvider>>,
    /// 當 skill 沒在 `skills` map 內時用這個。預設 = `agent`,但 agent 是
    /// `bash-cli-agent` 那種「自己會 spawn AI CLI 當 agent」型 provider 時
    /// 會自動 fallback 到 chat-only(claude-cli)避免:
    ///   bash-cli-agent → spawn claude → claude call mori skill polish →
    ///   PolishSkill.exec → bash-cli-agent → spawn claude → … (無限遞迴)
    pub skill_fallback: Arc<dyn LlmProvider>,
}

impl Routing {
    /// 取 skill `name` 該用的 provider — 先看 override,再用 skill_fallback。
    pub fn skill_provider(&self, name: &str) -> Arc<dyn LlmProvider> {
        self.skills
            .get(name)
            .cloned()
            .unwrap_or_else(|| self.skill_fallback.clone())
    }

    /// 從 `~/.mori/config.json` 的 `routing` block 蓋出整套 routing。
    ///
    /// retry_cb 只會 attach 到構造出的 Groq 實例 — 即便 routing 用了多個
    /// provider,Groq 那個會收到 callback,其他不會。
    pub fn build_from_config(
        retry_cb: Option<groq::RetryCallback>,
    ) -> anyhow::Result<Self> {
        let default = read_provider_config();
        let cfg = read_routing_config();

        let agent_name = cfg
            .agent
            .clone()
            .unwrap_or_else(|| default.clone());

        // 收集所有需要構造的 provider names — agent + 所有 skill override values
        let mut needed: HashSet<String> = HashSet::new();
        needed.insert(agent_name.clone());
        for v in cfg.skills.values() {
            needed.insert(v.clone());
        }

        // 蓋出每個 unique provider。retry_cb 只發給 groq;其他 None。
        let mut built: HashMap<String, Arc<dyn LlmProvider>> = HashMap::new();
        for name in &needed {
            let cb = if name == "groq" { retry_cb.clone() } else { None };
            let p = build_named_provider(name, cb)
                .with_context(|| format!("build provider '{}'", name))?;
            built.insert(name.clone(), p);
        }

        let agent = built
            .get(&agent_name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("agent provider '{}' not built", agent_name))?;

        if !agent.supports_tool_calling() {
            tracing::warn!(
                agent = %agent_name,
                "configured agent provider does not support tool calling — \
                 main agent loop will get text-only fallback responses, skill \
                 dispatch will not fire. Use 'groq' or 'ollama' for agent, \
                 keep chat-only providers (claude-cli) for skill overrides."
            );
        }

        let skills: HashMap<String, Arc<dyn LlmProvider>> = cfg
            .skills
            .iter()
            .filter_map(|(skill_name, provider_name)| {
                built
                    .get(provider_name)
                    .map(|p| (skill_name.clone(), p.clone()))
            })
            .collect();

        // Anti-recursion guard:bash-cli 系列 provider(bash-cli-agent /
        // gemini-bash / codex-bash)是「spawn 外部 AI CLI」型,若當 skill
        // provider 會無限遞迴:agent spawn gemini → gemini call mori skill
        // translate → TranslateSkill 用 gemini-bash → 又 spawn gemini →…。
        // 自動 fallback 到 claude-cli(chat-only,不再 spawn)。
        // User 仍可在 routing.skills 顯式覆寫個別 skill。
        let is_bash_cli = matches!(agent.name(), "bash-cli-agent" | "gemini-bash" | "codex-bash");
        let skill_fallback = if is_bash_cli {
            let fallback_name = "claude-cli";
            let p = match built.get(fallback_name) {
                Some(p) => p.clone(),
                None => {
                    let p = build_named_provider(fallback_name, None)
                        .with_context(|| {
                            "auto-build claude-cli as skill fallback for bash-cli-agent agent"
                        })?;
                    built.insert(fallback_name.into(), p.clone());
                    p
                }
            };
            tracing::warn!(
                agent = %agent_name,
                fallback = %fallback_name,
                "agent is bash-cli type — auto-fallback skills to '{}' to avoid recursion. \
                 Set routing.skills.<name> in config to override per skill.",
                fallback_name,
            );
            p
        } else {
            agent.clone()
        };

        tracing::info!(
            agent = %agent_name,
            agent_model = %agent.model(),
            agent_tools = agent.supports_tool_calling(),
            skill_fallback = %skill_fallback.name(),
            skill_overrides = ?cfg.skills,
            "routing built"
        );

        Ok(Self {
            agent,
            skills,
            skill_fallback,
        })
    }
}

/// `routing` block 解析快照 — 結構簡單,純 String 對應。給 IPC / log /
/// `Routing::build_from_config` 用。
#[derive(Default, Debug, Clone)]
pub struct RoutingConfig {
    /// `routing.agent`(可選),沒設就退化成 `provider`
    pub agent: Option<String>,
    /// `routing.skills` 的 skill→provider 對應表
    pub skills: HashMap<String, String>,
}

fn read_provider_config() -> String {
    mori_config_path()
        .as_deref()
        .and_then(|p| groq::read_json_pointer(p, "/provider"))
        .unwrap_or_else(|| "groq".to_string())
}

/// 讀 `routing.agent` + `routing.skills` 子物件。沒檔案 / 沒 routing /
/// 解析失敗都回 default(空 routing,等於沿用 provider 行為)。
pub fn read_routing_config() -> RoutingConfig {
    match mori_config_path() {
        Some(path) => read_routing_config_at(&path),
        None => RoutingConfig::default(),
    }
}

/// 純函式版本(可測):從指定 path 讀 routing block。
pub fn read_routing_config_at(path: &std::path::Path) -> RoutingConfig {
    let Ok(text) = std::fs::read_to_string(path) else {
        return RoutingConfig::default();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return RoutingConfig::default();
    };

    let agent = json
        .pointer("/routing/agent")
        .and_then(|v| v.as_str())
        .map(String::from);

    let skills: HashMap<String, String> = json
        .pointer("/routing/skills")
        .and_then(|v| v.as_object())
        .map(|map| {
            map.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();

    RoutingConfig { agent, skills }
}

#[cfg(test)]
mod routing_tests {
    use super::*;
    use tempfile::tempdir;

    fn write_config(json: &str) -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), json).unwrap();
        dir
    }

    #[test]
    fn nonexistent_file_returns_default() {
        let dir = tempdir().unwrap();
        let cfg = read_routing_config_at(&dir.path().join("nope.json"));
        assert!(cfg.agent.is_none());
        assert!(cfg.skills.is_empty());
    }

    #[test]
    fn malformed_json_returns_default() {
        let dir = write_config("{ invalid json");
        let cfg = read_routing_config_at(&dir.path().join("config.json"));
        assert!(cfg.agent.is_none());
        assert!(cfg.skills.is_empty());
    }

    #[test]
    fn missing_routing_block_returns_default() {
        let dir = write_config(r#"{"provider":"groq"}"#);
        let cfg = read_routing_config_at(&dir.path().join("config.json"));
        assert!(cfg.agent.is_none());
        assert!(cfg.skills.is_empty());
    }

    #[test]
    fn agent_only_no_skills() {
        let dir = write_config(r#"{"routing":{"agent":"ollama"}}"#);
        let cfg = read_routing_config_at(&dir.path().join("config.json"));
        assert_eq!(cfg.agent.as_deref(), Some("ollama"));
        assert!(cfg.skills.is_empty());
    }

    #[test]
    fn skills_only_no_agent() {
        let dir = write_config(
            r#"{"routing":{"skills":{"translate":"claude-cli","polish":"ollama"}}}"#,
        );
        let cfg = read_routing_config_at(&dir.path().join("config.json"));
        assert!(cfg.agent.is_none());
        assert_eq!(cfg.skills.len(), 2);
        assert_eq!(cfg.skills.get("translate").map(String::as_str), Some("claude-cli"));
        assert_eq!(cfg.skills.get("polish").map(String::as_str), Some("ollama"));
    }

    #[test]
    fn null_agent_treated_as_unset() {
        let dir = write_config(r#"{"routing":{"agent":null,"skills":{}}}"#);
        let cfg = read_routing_config_at(&dir.path().join("config.json"));
        assert!(cfg.agent.is_none());
    }

    #[test]
    fn full_routing_block() {
        let dir = write_config(
            r#"{
                "provider":"groq",
                "routing":{
                    "agent":"groq",
                    "skills":{
                        "translate":"claude-cli",
                        "polish":"claude-cli",
                        "summarize":"ollama"
                    }
                }
            }"#,
        );
        let cfg = read_routing_config_at(&dir.path().join("config.json"));
        assert_eq!(cfg.agent.as_deref(), Some("groq"));
        assert_eq!(cfg.skills.len(), 3);
    }

    #[test]
    fn non_string_skill_value_filtered_out() {
        // 例如 user 誤打成 number — 不該炸,只 skip 那個 key
        let dir = write_config(r#"{"routing":{"skills":{"a":"groq","b":42}}}"#);
        let cfg = read_routing_config_at(&dir.path().join("config.json"));
        assert_eq!(cfg.skills.len(), 1);
        assert!(cfg.skills.contains_key("a"));
        assert!(!cfg.skills.contains_key("b"));
    }

    // ─── 5J: resolve_api_key 與 gemini provider 常數 ───────────────────────

    /// 把 env var 暫存 / 還原，避免 test 之間互相污染。
    /// 留意：cargo test 預設多 thread,呼叫端要自己保證 key name 在不同 test 互不相干
    /// （這 5J 系列 test 都用獨立 key name 解決）。
    struct EnvGuard {
        key: String,
        prev: Option<String>,
    }
    impl EnvGuard {
        fn set(key: &str, value: &str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::set_var(key, value);
            Self { key: key.into(), prev }
        }
        fn unset(key: &str) -> Self {
            let prev = std::env::var(key).ok();
            std::env::remove_var(key);
            Self { key: key.into(), prev }
        }
    }
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.prev {
                Some(v) => std::env::set_var(&self.key, v),
                None => std::env::remove_var(&self.key),
            }
        }
    }

    #[test]
    fn resolve_api_key_env_var_wins() {
        let _g = EnvGuard::set("MORI_TEST_KEY_ENV_WINS", "from-env");
        let dir = write_config(r#"{"api_keys":{"MORI_TEST_KEY_ENV_WINS":"from-config"}}"#);
        let got = resolve_api_key_at(Some(&dir.path().join("config.json")), "MORI_TEST_KEY_ENV_WINS");
        assert_eq!(got.as_deref(), Some("from-env"));
    }

    #[test]
    fn resolve_api_key_falls_back_to_config() {
        let _g = EnvGuard::unset("MORI_TEST_KEY_CONFIG_FALLBACK");
        let dir = write_config(r#"{"api_keys":{"MORI_TEST_KEY_CONFIG_FALLBACK":"from-config"}}"#);
        let got = resolve_api_key_at(Some(&dir.path().join("config.json")), "MORI_TEST_KEY_CONFIG_FALLBACK");
        assert_eq!(got.as_deref(), Some("from-config"));
    }

    #[test]
    fn resolve_api_key_empty_env_falls_back() {
        // 空字串 env var 應視為未設,讓 config 接手
        let _g = EnvGuard::set("MORI_TEST_KEY_EMPTY_ENV", "");
        let dir = write_config(r#"{"api_keys":{"MORI_TEST_KEY_EMPTY_ENV":"from-config"}}"#);
        let got = resolve_api_key_at(Some(&dir.path().join("config.json")), "MORI_TEST_KEY_EMPTY_ENV");
        assert_eq!(got.as_deref(), Some("from-config"));
    }

    #[test]
    fn resolve_api_key_empty_config_value_returns_none() {
        let _g = EnvGuard::unset("MORI_TEST_KEY_EMPTY_CONFIG");
        let dir = write_config(r#"{"api_keys":{"MORI_TEST_KEY_EMPTY_CONFIG":""}}"#);
        let got = resolve_api_key_at(Some(&dir.path().join("config.json")), "MORI_TEST_KEY_EMPTY_CONFIG");
        assert!(got.is_none());
    }

    #[test]
    fn resolve_api_key_missing_returns_none() {
        let _g = EnvGuard::unset("MORI_TEST_KEY_MISSING");
        let dir = write_config(r#"{"api_keys":{}}"#);
        let got = resolve_api_key_at(Some(&dir.path().join("config.json")), "MORI_TEST_KEY_MISSING");
        assert!(got.is_none());
    }

    #[test]
    fn resolve_api_key_no_config_no_env() {
        let _g = EnvGuard::unset("MORI_TEST_KEY_NO_CONFIG");
        let got = resolve_api_key_at(None, "MORI_TEST_KEY_NO_CONFIG");
        assert!(got.is_none());
    }

    #[test]
    fn gemini_defaults_are_openai_compat_endpoint() {
        // OpenAI-compat 端點長相,不能變成原生 Google AI Studio 路徑
        assert!(GEMINI_DEFAULT_API_BASE.contains("generativelanguage.googleapis.com"));
        assert!(GEMINI_DEFAULT_API_BASE.contains("openai"));
        // model 預設應是個非空字串
        assert!(!GEMINI_DEFAULT_MODEL.is_empty());
    }

    // `build_named_provider("gemini", ...)` 讀 GEMINI_API_KEY env + $HOME。
    // env var 是 process-global,cargo test 預設多 thread 會 race。
    // 用 Mutex 序列化所有會動這兩個 env 的 test。
    static GEMINI_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn build_named_provider_gemini_fails_without_key() {
        let _lock = GEMINI_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::unset("GEMINI_API_KEY");
        // 暫時把 HOME 指到一個沒有 .mori/config.json 的 tempdir
        let home_dir = tempdir().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home_dir.path());
        let result = build_named_provider("gemini", None);
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        // Arc<dyn LlmProvider> 沒 Debug,不能用 expect_err
        let err = match result {
            Ok(_) => panic!("expected build_named_provider(\"gemini\") to fail without GEMINI_API_KEY"),
            Err(e) => e,
        };
        let msg = format!("{err:#}");
        assert!(msg.contains("GEMINI_API_KEY"), "error should mention GEMINI_API_KEY, got: {msg}");
    }

    #[test]
    fn build_named_provider_gemini_succeeds_with_env_key() {
        let _lock = GEMINI_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g = EnvGuard::set("GEMINI_API_KEY", "test-key-12345");
        let home_dir = tempdir().unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home_dir.path());
        let result = build_named_provider("gemini", None);
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let p = result.expect("should build with env key");
        assert_eq!(p.name(), "gemini");
        assert!(p.supports_tool_calling());
        assert_eq!(p.model(), GEMINI_DEFAULT_MODEL);
    }

    // 5N: 自訂 OpenAI-compat 端點 — 把 ZEROTYPE_AIPROMPT_* frontmatter 鍵
    // 替換成 config.json `providers.<name>` 機制。env / config_path / HOME
    // 都是 process-global,跟 gemini 系列 test 共用 lock 避免 race。
    #[test]
    fn build_named_provider_custom_openai_compat_from_config() {
        let _lock = GEMINI_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home_dir = tempdir().unwrap();
        std::fs::create_dir_all(home_dir.path().join(".mori")).unwrap();
        std::fs::write(
            home_dir.path().join(".mori/config.json"),
            r#"{
              "providers": {
                "azure-gpt41": {
                  "api_base": "https://example.openai.azure.com/openai/v1",
                  "api_key_env": "MORI_TEST_AZURE_KEY",
                  "model": "gpt-4.1"
                }
              }
            }"#,
        )
        .unwrap();
        let _g_key = EnvGuard::set("MORI_TEST_AZURE_KEY", "azure-test-key");
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home_dir.path());
        let result = build_named_provider("azure-gpt41", None);
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let p = result.expect("custom provider should build from config.json");
        assert_eq!(p.name(), "azure-gpt41", "name = config.json provider key");
        assert_eq!(p.model(), "gpt-4.1");
        assert!(p.supports_tool_calling());
    }

    #[test]
    fn build_named_provider_unknown_no_api_base_errors() {
        let _lock = GEMINI_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home_dir = tempdir().unwrap();
        std::fs::create_dir_all(home_dir.path().join(".mori")).unwrap();
        // 故意沒設 providers.foo.api_base
        std::fs::write(
            home_dir.path().join(".mori/config.json"),
            r#"{"providers": {"foo": {"model": "anything"}}}"#,
        )
        .unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home_dir.path());
        let result = build_named_provider("foo", None);
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let err = match result {
            Ok(_) => panic!("expected error for unknown provider without api_base"),
            Err(e) => format!("{e:#}"),
        };
        assert!(err.contains("unknown provider"), "got: {err}");
        assert!(err.contains("api_base"), "error should hint api_base setup, got: {err}");
    }

    #[test]
    fn build_named_provider_custom_missing_key_errors() {
        let _lock = GEMINI_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _g_key = EnvGuard::unset("MORI_TEST_NO_KEY_ENV");
        let home_dir = tempdir().unwrap();
        std::fs::create_dir_all(home_dir.path().join(".mori")).unwrap();
        std::fs::write(
            home_dir.path().join(".mori/config.json"),
            r#"{
              "providers": {
                "azure-x": {
                  "api_base": "https://example.openai.azure.com/openai/v1",
                  "api_key_env": "MORI_TEST_NO_KEY_ENV",
                  "model": "gpt-4.1"
                }
              }
            }"#,
        )
        .unwrap();
        let prev_home = std::env::var("HOME").ok();
        std::env::set_var("HOME", home_dir.path());
        let result = build_named_provider("azure-x", None);
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        let err = match result {
            Ok(_) => panic!("expected error when api_key cannot be resolved"),
            Err(e) => format!("{e:#}"),
        };
        assert!(err.contains("api_key"), "error should mention api_key, got: {err}");
    }
}

use anyhow::Context as _;

/// 目前生效的 chat provider 設定快照。給 UI / IPC / warm-up 用,
/// 避免各處重複讀 config + 各自落 fallback。
#[derive(Debug, Clone)]
pub struct ProviderSnapshot {
    pub name: String,
    pub model: String,
    /// Ollama 才有;Groq/雲端 provider 為 None。
    pub base_url: Option<String>,
}

pub fn active_chat_provider_snapshot() -> ProviderSnapshot {
    // 5A-3 起:agent 走 `routing.agent`(若設)→ `provider`(若設)→ "groq"
    let routing = read_routing_config();
    let default = read_provider_config();
    let active = routing.agent.unwrap_or_else(|| default.clone());

    match active.as_str() {
        "ollama" => {
            let base_url = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/ollama/base_url"))
                .unwrap_or_else(|| ollama::OllamaProvider::DEFAULT_BASE_URL.to_string());
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/ollama/model"))
                .unwrap_or_else(|| ollama::OllamaProvider::DEFAULT_MODEL.to_string());
            ProviderSnapshot {
                name: "ollama".into(),
                model,
                base_url: Some(base_url),
            }
        }
        "claude-cli" => {
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/claude-cli/model"))
                .unwrap_or_else(|| "(claude-cli default)".to_string());
            ProviderSnapshot {
                name: "claude-cli".into(),
                model,
                base_url: None,
            }
        }
        "claude-bash" => {
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/claude-bash/model"))
                .unwrap_or_else(|| "(agent CLI default)".to_string());
            ProviderSnapshot { name: "claude-bash".into(), model, base_url: None }
        }
        "gemini-bash" => {
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/gemini-bash/model"))
                .unwrap_or_else(|| "(gemini default)".to_string());
            ProviderSnapshot { name: "gemini-bash".into(), model, base_url: None }
        }
        "codex-bash" => {
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/codex-bash/model"))
                .unwrap_or_else(|| "(codex default)".to_string());
            ProviderSnapshot { name: "codex-bash".into(), model, base_url: None }
        }
        "gemini-cli" => {
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/gemini-cli/model"))
                .unwrap_or_else(|| "(gemini default)".to_string());
            ProviderSnapshot { name: "gemini-cli".into(), model, base_url: None }
        }
        "codex-cli" => {
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/codex-cli/model"))
                .unwrap_or_else(|| "(codex default)".to_string());
            ProviderSnapshot { name: "codex-cli".into(), model, base_url: None }
        }
        "gemini" => {
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/gemini/model"))
                .unwrap_or_else(|| GEMINI_DEFAULT_MODEL.to_string());
            ProviderSnapshot { name: "gemini".into(), model, base_url: None }
        }
        _ => {
            let model = mori_config_path()
                .as_deref()
                .and_then(|p| groq::read_json_pointer(p, "/providers/groq/model"))
                .unwrap_or_else(|| groq::GroqProvider::DEFAULT_MODEL.to_string());
            ProviderSnapshot {
                name: "groq".into(),
                model,
                base_url: None,
            }
        }
    }
}

/// 啟動時的 best-effort warm-up:若使用者把 `provider` 設成 ollama,
/// 背景發一個 1-token 的 chat 把模型載進 RAM,使用者第一次按熱鍵時就不用
/// 等 cold start(qwen3:8b 5.2GB 在 Intel CPU 沒 GPU 加速可能要分鐘級)。
///
/// Provider 是 groq 時直接 no-op(網路 LLM 沒 cold start)。
/// 失敗無聲忽略 — UI 想知道狀態的話走 mori-tauri 那邊發事件版本。
pub async fn warm_up_provider() {
    let snap = active_chat_provider_snapshot();
    if snap.name != "ollama" {
        return;
    }
    if let Some(base_url) = snap.base_url {
        if let Err(e) = ollama::OllamaProvider::warm_up(&base_url, &snap.model).await {
            tracing::debug!(?e, "ollama warm-up failed (non-fatal)");
        }
    }
}

fn mori_config_path() -> Option<std::path::PathBuf> {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".mori").join("config.json"))
}

/// 一則訊息。
///
/// 用 `Option<String>` 給 content 是因為 assistant 在發起 tool_call 時可能
/// 沒文字內容。`tool_calls` 只在 assistant 發起時非空。`tool_call_id` + `name`
/// 只在 role="tool" 時用,把回傳結果連回對應的 tool_call。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: None,
            name: None,
        }
    }

    pub fn assistant_with_tool_calls(
        content: Option<String>,
        tool_calls: Vec<ToolCall>,
    ) -> Self {
        Self {
            role: "assistant".into(),
            content,
            tool_calls,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn tool_result(
        call_id: impl Into<String>,
        name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
            name: Some(name.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// API 給的唯一 id(回傳 tool 結果要 reference 它)
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    /// LLM 自由文字回應(若沒呼叫 tool 或 mid-thought)
    pub content: Option<String>,
    /// LLM 決定呼叫的 tools
    pub tool_calls: Vec<ToolCall>,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Provider 識別名(groq / ollama / openai / anthropic / ...)
    fn name(&self) -> &'static str;

    /// 模型 id
    fn model(&self) -> &str;

    /// 這個 provider 是否能 dispatch tool calls。
    /// `true`(default):可以當主 agent loop 的 provider。
    /// `false`:只能拿來做 skill 內部 chat — 例如 Claude CLI 沒有 OpenAI 風格
    /// 的 function-calling channel。Routing 在挑 agent provider 時會檢查這個
    /// 旗標,若使用者誤把 chat-only provider 配給 agent 會 warn(或 fail)。
    fn supports_tool_calling(&self) -> bool {
        true
    }

    /// 跑一輪 chat completion。
    async fn chat(
        &self,
        messages: Vec<ChatMessage>,
        tools: Vec<ToolDefinition>,
    ) -> Result<ChatResponse>;
}

// transcribe 已從這個 trait 移到 [`super::transcribe::TranscriptionProvider`]
// (5C)— 該 trait 跟 chat 解耦,LocalWhisper 可以只做 STT 不必假裝會 chat。
