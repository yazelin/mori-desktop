//! `AnnuliMemoryStore` — Wave 4 新實作,把 `MemoryStore` trait wrap 到 annuli HTTP API。
//!
//! 設計依據 [`docs/WAVE-4-DESIGN.md`](../../../../docs/WAVE-4-DESIGN.md) Q1+Q2+Q3:
//!
//! - **Q1**:`MemoryIndexEntry ← § section in MEMORY.md`。`id = sluggified header`,
//!   `name = header`,`memory_type = Other("vault_section")`。`voice_dict` 5E-3
//!   走 header convention(`## § voice_dict: <X>`),沒匹配回 [] fallback。
//! - **Q2**:`write` → annuli `POST /spirits/<x>/memory/section`(Wave 4 prep PR
//!   merged in annuli `629377d`)。需 X-Soul-Token,token 從 `AnnuliClient` config 帶。
//! - **Q3**:`delete` → `Err("use curator review")`(Wave 4 暫不實作,等 Wave 5+
//!   curator UI)。
//!
//! 跟 `LocalMarkdownMemoryStore` 共存:`mori-tauri/src/main.rs` 啟動時 config
//! 沒 `annuli.enabled=true` → 用 LocalMarkdown;有 → 用 Annuli。

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use futures_util::stream::{self, BoxStream};
use std::sync::Arc;

use crate::annuli::{AnnuliClient, AnnuliError};

use super::{Memory, MemoryEvent, MemoryIndexEntry, MemoryStore, MemoryType};

pub struct AnnuliMemoryStore {
    client: Arc<AnnuliClient>,
}

impl AnnuliMemoryStore {
    pub fn new(client: Arc<AnnuliClient>) -> Self {
        Self { client }
    }

    /// Slugify section header for use as `Memory.id`. Keep alphanumeric + dash +
    /// underscore + dot + tilde + Chinese chars; others → `-`. Lower-case ascii.
    fn slugify(header: &str) -> String {
        let mut out = String::with_capacity(header.len());
        for c in header.chars() {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
                out.push(c.to_ascii_lowercase());
            } else if !c.is_ascii() {
                // 保留中文等 non-ASCII
                out.push(c);
            } else {
                // 其他 ASCII(空格 / 標點)→ -
                if !out.ends_with('-') {
                    out.push('-');
                }
            }
        }
        out.trim_matches('-').to_string()
    }

    /// 反向:從 slug 還原 header(精確還原不可能,所以用 best-effort lookup)。
    async fn header_by_slug(&self, slug: &str) -> Result<Option<String>> {
        let headers = self.list_section_headers().await?;
        Ok(headers.into_iter().find(|h| Self::slugify(h) == slug))
    }

    /// 解析 MEMORY.md 的所有 `## § <header>` lines via annuli `GET /memory` endpoint。
    /// `include_body=false` 只回 header(輕量,read_index 用)。
    async fn list_section_headers(&self) -> Result<Vec<String>> {
        let sections = self
            .client
            .list_memory_sections(false)
            .await
            .map_err(Self::map_annuli_error)?;
        Ok(sections.into_iter().map(|s| s.header).collect())
    }

    /// 拿某個 header 對應的 body(走 include_body=true)。
    async fn fetch_section_body(&self, target_header: &str) -> Result<Option<String>> {
        let sections = self
            .client
            .list_memory_sections(true)
            .await
            .map_err(Self::map_annuli_error)?;
        Ok(sections
            .into_iter()
            .find(|s| s.header == target_header)
            .and_then(|s| s.body))
    }

    fn map_annuli_error(e: AnnuliError) -> anyhow::Error {
        anyhow!("annuli HTTP: {}", e)
    }
}

#[async_trait]
impl MemoryStore for AnnuliMemoryStore {
    async fn read_index(&self) -> Result<Vec<MemoryIndexEntry>> {
        let headers = self.list_section_headers().await?;
        Ok(headers
            .into_iter()
            .map(|h| {
                // 解 type prefix:`preference: 喜歡冷咖啡` → MemoryType::Preference + name
                let (memory_type, name) = parse_header_type(&h);
                MemoryIndexEntry {
                    id: Self::slugify(&h),
                    name,
                    description: String::new(),
                    memory_type,
                }
            })
            .collect())
    }

    async fn read(&self, id: &str) -> Result<Option<Memory>> {
        let Some(header) = self.header_by_slug(id).await? else {
            return Ok(None);
        };
        let Some(body) = self.fetch_section_body(&header).await? else {
            return Ok(None);
        };
        let (memory_type, name) = parse_header_type(&header);
        let now = Utc::now();
        Ok(Some(Memory {
            id: id.to_string(),
            name,
            description: String::new(),
            memory_type,
            created: now,
            last_used: now,
            body,
        }))
    }

    async fn write(&self, memory: Memory) -> Result<()> {
        // Q2 (b):POST /spirits/<x>/memory/section
        // 用 memory.name 當 header,memory.body 當 body。type 資訊放 header convention:
        //   "## § voice_dict: <name>"  / "## § preference: <name>" / ...(Q1)
        let type_prefix = match &memory.memory_type {
            MemoryType::Other(s) if s == "vault_section" => String::new(),
            t => format!("{}: ", t.as_str()),
        };
        let header = format!("{}{}", type_prefix, memory.name);

        self.client
            .append_memory_section(&header, &memory.body)
            .await
            .map_err(Self::map_annuli_error)
            .with_context(|| format!("annuli POST /memory/section header={}", header))?;
        tracing::info!(id = %memory.id, "memory written via annuli");
        Ok(())
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Memory>> {
        // 用 annuli events FTS 找(Wave 4 phase — 只搜對話 events,不搜 MEMORY.md)
        // trigram tokenizer 需 ≥3 字 — 短 query 直接回 []
        if query.chars().count() < 3 {
            return Ok(Vec::new());
        }
        let events = self
            .client
            .search_events(query, limit as u32)
            .await
            .map_err(Self::map_annuli_error)?;
        Ok(events
            .into_iter()
            .filter_map(|ev| {
                // 從 event 抽出 text(只摘 chat kind 給 search 結果)
                let text = ev.data.get("text").and_then(|v| v.as_str())?;
                Some(Memory {
                    id: ev.id.as_ref().map(|(d, n)| format!("{}-{}", d, n)).unwrap_or_default(),
                    name: ev.kind.clone(),
                    description: format!("event @ {}", ev.ts.format("%Y-%m-%d %H:%M")),
                    memory_type: MemoryType::Other("event".into()),
                    created: ev.ts.with_timezone(&Utc),
                    last_used: ev.ts.with_timezone(&Utc),
                    body: text.to_string(),
                })
            })
            .collect())
    }

    async fn delete(&self, _id: &str) -> Result<()> {
        // Q3 (b):暫不實作,叫 caller 走 curator review。
        Err(anyhow!(
            "AnnuliMemoryStore.delete 暫不支援 — 走 `annuli curator dry-run` + yaml approve + `apply` 流程"
        ))
    }

    fn observe(&self) -> BoxStream<'static, MemoryEvent> {
        // Wave 4 暫不支援 push events(HTTP polling 才有,等 Wave 5+ Server-Sent Events)
        Box::pin(stream::empty())
    }

    async fn list_by_types(&self, types: &[MemoryType]) -> Result<Vec<Memory>> {
        if types.is_empty() {
            return Ok(Vec::new());
        }
        // Q1 (a) + fallback:走 `## § voice_dict: X` header convention
        let sections = self
            .client
            .list_memory_sections(true)
            .await
            .map_err(Self::map_annuli_error)?;
        let now = Utc::now();
        let mut out = Vec::new();
        for sec in sections {
            let (mtype, name) = parse_header_type(&sec.header);
            if !types.contains(&mtype) {
                continue;
            }
            let Some(body) = sec.body else { continue };
            out.push(Memory {
                id: Self::slugify(&sec.header),
                name,
                description: String::new(),
                memory_type: mtype,
                created: now,
                last_used: now,
                body,
            });
        }
        Ok(out)
    }
}

/// 解析 § header `<type>: <name>` convention(Q1 (a))。
///
/// - `"preference: 喜歡冷咖啡"` → `(MemoryType::Preference, "喜歡冷咖啡")`
/// - `"voice_dict: Annuli"` → `(MemoryType::VoiceDict, "Annuli")`
/// - `"2026-05-14"` 或無冒號 → `(MemoryType::Other("vault_section"), 原 header)`
fn parse_header_type(header: &str) -> (MemoryType, String) {
    if let Some((prefix, rest)) = header.split_once(':') {
        let prefix = prefix.trim();
        let name = rest.trim().to_string();
        // 只認 known type slug(避免「日期: 標題」型 header 被誤判)
        let known = ["user_identity", "preference", "skill_outcome", "project", "reference", "voice_dict"];
        if known.contains(&prefix) {
            return (MemoryType::parse(prefix), name);
        }
    }
    (MemoryType::Other("vault_section".into()), header.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_ascii() {
        assert_eq!(AnnuliMemoryStore::slugify("Hello World"), "hello-world");
        assert_eq!(AnnuliMemoryStore::slugify("foo  bar"), "foo-bar");
        assert_eq!(AnnuliMemoryStore::slugify("a/b/c"), "a-b-c");
        assert_eq!(AnnuliMemoryStore::slugify("---"), "");
    }

    #[test]
    fn slugify_preserves_chinese() {
        assert_eq!(AnnuliMemoryStore::slugify("森林之靈"), "森林之靈");
        assert_eq!(AnnuliMemoryStore::slugify("2026-05-14 喝水提醒"), "2026-05-14-喝水提醒");
    }

    #[test]
    fn slugify_lowers_ascii_only() {
        assert_eq!(AnnuliMemoryStore::slugify("ABC123"), "abc123");
    }

    // === parse_header_type ===

    #[test]
    fn parse_header_with_known_type_prefix() {
        let (t, name) = parse_header_type("preference: 喜歡冷咖啡");
        assert!(matches!(t, MemoryType::Preference));
        assert_eq!(name, "喜歡冷咖啡");

        let (t, name) = parse_header_type("voice_dict: Annuli");
        assert!(matches!(t, MemoryType::VoiceDict));
        assert_eq!(name, "Annuli");
    }

    #[test]
    fn parse_header_date_format_keeps_as_vault_section() {
        // `2026-05-14` 不該被當成 unknown type
        let (t, name) = parse_header_type("2026-05-14");
        assert!(matches!(t, MemoryType::Other(ref s) if s == "vault_section"));
        assert_eq!(name, "2026-05-14");
    }

    #[test]
    fn parse_header_with_unknown_prefix_keeps_full() {
        // 沒在 known list → 整 header 留著當 name
        let (t, name) = parse_header_type("weird_thing: x");
        assert!(matches!(t, MemoryType::Other(ref s) if s == "vault_section"));
        assert_eq!(name, "weird_thing: x");
    }
}
