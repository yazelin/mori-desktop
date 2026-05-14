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
    /// 邏輯:fetch all sections,找第一個 slug 相同的。給 read(id) 用。
    async fn header_by_slug(&self, slug: &str) -> Result<Option<String>> {
        let sections = self.parse_memory_md_sections().await?;
        Ok(sections.into_iter().find(|h| Self::slugify(h) == slug).map(|h| h.clone()))
    }

    /// 解析 MEMORY.md 的所有 `## § <header>` lines。
    /// 用 annuli `GET /spirits/<x>/soul` 拿不到 MEMORY.md — 我們需要另外的方法。
    /// 暫時走 hack:annuli 沒給 `GET /memory` endpoint。Wave 4 後續可加;現在透過
    /// `search_events` 不行(那是 events 不是 MEMORY)。
    ///
    /// **TODO Wave 4 後續**:annuli 加 `GET /spirits/<x>/memory` route。本實作目前先
    /// 用 events 假裝(events 包含 user_remember kind 對應寫入)— 但這走的是 event log
    /// 不是 MEMORY.md。短期權宜,Wave 5+ 加 read endpoint 才完整。
    async fn parse_memory_md_sections(&self) -> Result<Vec<String>> {
        // Wave 4 limitation:annuli 還沒 `GET /spirits/<x>/memory` route。
        // 這裡先回 [] — search() 跟 list_by_types() 直接走 events FTS 跟 fallback。
        // read_index() 因此暫時也回 []。
        //
        // 加 endpoint 後改成:
        //   let memory_md = self.client.get_memory_md().await?;
        //   let re = Regex::new(r"(?m)^## § (.+?)$").unwrap();
        //   Ok(re.captures_iter(&memory_md).map(|c| c[1].trim().to_string()).collect())
        Ok(Vec::new())
    }

    fn map_annuli_error(e: AnnuliError) -> anyhow::Error {
        anyhow!("annuli HTTP: {}", e)
    }
}

#[async_trait]
impl MemoryStore for AnnuliMemoryStore {
    async fn read_index(&self) -> Result<Vec<MemoryIndexEntry>> {
        let headers = self.parse_memory_md_sections().await?;
        Ok(headers
            .into_iter()
            .map(|h| MemoryIndexEntry {
                id: Self::slugify(&h),
                name: h.clone(),
                description: String::new(),
                memory_type: MemoryType::Other("vault_section".into()),
            })
            .collect())
    }

    async fn read(&self, id: &str) -> Result<Option<Memory>> {
        // Wave 4 暫不實作(等 annuli 加 `GET /memory` route);回 None,callers 該 graceful 處理
        let _ = id;
        let Some(_header) = self.header_by_slug(id).await? else {
            return Ok(None);
        };
        // 假設將來有 `get_memory_section(header) -> body` API,這裡先回 None。
        Ok(None)
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
        // Q1 fallback:走 `## § voice_dict: X` header convention。沒 endpoint 暫回 [] 不破。
        let _ = types;
        // Wave 4 limit:沒 GET /memory endpoint,只能回 [](Voice cleanup 會 fallback 空 dict)。
        Ok(Vec::new())
    }
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
}
