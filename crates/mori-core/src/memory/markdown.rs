//! 把 memory 存成 markdown 檔(對齊 Claude Code auto-memory 慣例)。
//!
//! Layout:
//! ```text
//! ~/.mori/memory/
//! ├── MEMORY.md                  ← 索引,每行一個 memory + 一句描述
//! ├── user_preferences.md        ← frontmatter + body
//! ├── project_xxx.md
//! └── ...
//! ```
//!
//! 索引行格式:
//! ```text
//! - [Display Name](file_id.md) — short description
//! ```
//!
//! Memory 檔格式:
//! ```text
//! ---
//! name: User prefers terse responses
//! description: Established 2026-05-06
//! type: preference
//! created: 2026-05-06T14:23:00Z
//! last_used: 2026-05-07T09:15:00Z
//! ---
//!
//! User prefers responses without:
//! - Unnecessary preamble
//! ```

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Utc;
use futures_util::stream::{self, BoxStream};
use std::path::{Path, PathBuf};

use super::{Memory, MemoryEvent, MemoryIndexEntry, MemoryStore, MemoryType};

pub struct LocalMarkdownMemoryStore {
    root: PathBuf,
}

impl LocalMarkdownMemoryStore {
    /// 預設位置:`~/.mori/memory/`
    pub fn default_root() -> Result<PathBuf> {
        let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"))?;
        Ok(PathBuf::from(home).join(".mori").join("memory"))
    }

    pub fn new(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&root)?;
        let index = root.join("MEMORY.md");
        if !index.exists() {
            std::fs::write(&index, "# Mori Memory Index\n\n")?;
        }
        Ok(Self { root })
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    fn index_path(&self) -> PathBuf {
        self.root.join("MEMORY.md")
    }

    fn memory_path(&self, id: &str) -> PathBuf {
        self.root.join(format!("{id}.md"))
    }

    /// 把 core memory 直接攤平成一段純文字,適合塞進 system prompt。
    ///
    /// Phase 1C 為了讓 LLM「知道你是誰」,我們會在每次 chat 時讀整個索引 +
    /// 全部 memory 內容當作背景。如果 memory 多了會嫌長,phase 5+ 再做
    /// 「LLM 自己挑相關 memory」邏輯。
    pub fn read_all_as_context(&self) -> Result<String> {
        let entries = blocking_read_index(&self.index_path())?;
        if entries.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::new();
        out.push_str("# 你已知關於使用者的事\n\n");
        for entry in &entries {
            let path = self.memory_path(&entry.id);
            if let Ok(memory) = blocking_read_memory(&path) {
                out.push_str(&format!("## {} ({:?})\n", memory.name, memory.memory_type));
                out.push_str(&memory.body);
                out.push_str("\n\n");
            }
        }
        Ok(out)
    }
}

#[async_trait]
impl MemoryStore for LocalMarkdownMemoryStore {
    async fn read_index(&self) -> Result<Vec<MemoryIndexEntry>> {
        blocking_read_index(&self.index_path())
    }

    async fn read(&self, id: &str) -> Result<Option<Memory>> {
        let path = self.memory_path(id);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(blocking_read_memory(&path)?))
    }

    async fn write(&self, mut memory: Memory) -> Result<()> {
        if memory.created.timestamp() == 0 {
            memory.created = Utc::now();
        }
        memory.last_used = Utc::now();

        let path = self.memory_path(&memory.id);
        std::fs::write(&path, format_memory(&memory)).context("write memory file")?;

        upsert_index_entry(
            &self.index_path(),
            &MemoryIndexEntry {
                id: memory.id.clone(),
                name: memory.name.clone(),
                description: memory.description.clone(),
                memory_type: memory.memory_type.clone(),
            },
        )?;

        tracing::info!(id = %memory.id, "memory written");
        Ok(())
    }

    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Memory>> {
        // Phase 1C: grep — 搜 name / description / body 是否含 query。
        // Phase 5+ 換成 sqlite-vec embedding。
        let needle = query.to_lowercase();
        let entries = blocking_read_index(&self.index_path())?;
        let mut out = Vec::new();
        for entry in entries {
            let path = self.memory_path(&entry.id);
            let mem = match blocking_read_memory(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };
            if mem.name.to_lowercase().contains(&needle)
                || mem.description.to_lowercase().contains(&needle)
                || mem.body.to_lowercase().contains(&needle)
            {
                out.push(mem);
                if out.len() >= limit {
                    break;
                }
            }
        }
        Ok(out)
    }

    async fn delete(&self, id: &str) -> Result<()> {
        let path = self.memory_path(id);
        if path.exists() {
            std::fs::remove_file(&path).context("remove memory file")?;
        }
        remove_index_entry(&self.index_path(), id)?;
        Ok(())
    }

    fn observe(&self) -> BoxStream<'static, MemoryEvent> {
        // Phase 1C: 不發 events。Phase 5+ 接 inotify / FSEvents 才有意義。
        Box::pin(stream::empty())
    }
}

// ─── 純檔案 I/O 與解析 ──────────────────────────────────────────────

fn blocking_read_index(path: &Path) -> Result<Vec<MemoryIndexEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(path).context("read index")?;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_start();
        if !line.starts_with("- [") {
            continue;
        }
        let after_open = match line.strip_prefix("- [") {
            Some(s) => s,
            None => continue,
        };
        let close_bracket = match after_open.find("](") {
            Some(i) => i,
            None => continue,
        };
        let name = &after_open[..close_bracket];
        let after_paren = &after_open[close_bracket + 2..];
        let close_paren = match after_paren.find(')') {
            Some(i) => i,
            None => continue,
        };
        let file = &after_paren[..close_paren];
        let id = file.strip_suffix(".md").unwrap_or(file).to_string();
        let rest = &after_paren[close_paren + 1..];
        let description = rest
            .trim_start_matches([' ', '—', '-', '–'])
            .trim()
            .to_string();
        out.push(MemoryIndexEntry {
            id,
            name: name.to_string(),
            description,
            memory_type: MemoryType::Other("unknown".into()),
        });
    }
    Ok(out)
}

fn blocking_read_memory(path: &Path) -> Result<Memory> {
    let text = std::fs::read_to_string(path).context("read memory file")?;
    let id = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    parse_memory(&id, &text)
}

fn parse_memory(id: &str, text: &str) -> Result<Memory> {
    let mut name = id.replace('_', " ");
    let mut description = String::new();
    let mut memory_type = MemoryType::Other("unknown".into());
    let mut created = Utc::now();
    let mut last_used = Utc::now();
    let body;

    if let Some(rest) = text.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n") {
            let header = &rest[..end];
            body = rest[end + 5..].trim().to_string();
            for line in header.lines() {
                let (key, value) = match line.split_once(':') {
                    Some((k, v)) => (k.trim(), v.trim().trim_matches('"').trim_matches('\'')),
                    None => continue,
                };
                match key {
                    "name" => name = value.to_string(),
                    "description" => description = value.to_string(),
                    "type" => memory_type = parse_memory_type(value),
                    "created" => {
                        if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(value) {
                            created = ts.with_timezone(&Utc);
                        }
                    }
                    "last_used" => {
                        if let Ok(ts) = chrono::DateTime::parse_from_rfc3339(value) {
                            last_used = ts.with_timezone(&Utc);
                        }
                    }
                    _ => {}
                }
            }
        } else {
            body = text.to_string();
        }
    } else {
        body = text.to_string();
    }

    Ok(Memory {
        id: id.to_string(),
        name,
        description,
        memory_type,
        created,
        last_used,
        body,
    })
}

fn parse_memory_type(s: &str) -> MemoryType {
    match s.to_lowercase().as_str() {
        "user_identity" | "user-identity" | "useridentity" => MemoryType::UserIdentity,
        "preference" => MemoryType::Preference,
        "skill_outcome" | "skill-outcome" | "skilloutcome" => MemoryType::SkillOutcome,
        "project" => MemoryType::Project,
        "reference" => MemoryType::Reference,
        other => MemoryType::Other(other.to_string()),
    }
}

fn format_memory(m: &Memory) -> String {
    let type_str = match &m.memory_type {
        MemoryType::UserIdentity => "user_identity".to_string(),
        MemoryType::Preference => "preference".to_string(),
        MemoryType::SkillOutcome => "skill_outcome".to_string(),
        MemoryType::Project => "project".to_string(),
        MemoryType::Reference => "reference".to_string(),
        MemoryType::Other(s) => s.clone(),
    };
    format!(
        "---\nname: {}\ndescription: {}\ntype: {}\ncreated: {}\nlast_used: {}\n---\n\n{}\n",
        m.name,
        m.description,
        type_str,
        m.created.to_rfc3339(),
        m.last_used.to_rfc3339(),
        m.body.trim_end()
    )
}

fn upsert_index_entry(index_path: &Path, entry: &MemoryIndexEntry) -> Result<()> {
    let mut lines: Vec<String> = if index_path.exists() {
        std::fs::read_to_string(index_path)?
            .lines()
            .map(String::from)
            .collect()
    } else {
        vec!["# Mori Memory Index".into(), String::new()]
    };
    let new_line = format!(
        "- [{}]({}.md) — {}",
        entry.name, entry.id, entry.description
    );

    let needle = format!("]({}.md)", entry.id);
    let mut replaced = false;
    for line in lines.iter_mut() {
        if line.contains(&needle) {
            *line = new_line.clone();
            replaced = true;
            break;
        }
    }
    if !replaced {
        if !lines.last().map(|l| l.is_empty()).unwrap_or(false) {
            lines.push(String::new());
        }
        lines.push(new_line);
    }
    std::fs::write(index_path, lines.join("\n") + "\n")?;
    Ok(())
}

fn remove_index_entry(index_path: &Path, id: &str) -> Result<()> {
    if !index_path.exists() {
        return Ok(());
    }
    let needle = format!("]({}.md)", id);
    let kept: Vec<String> = std::fs::read_to_string(index_path)?
        .lines()
        .filter(|l| !l.contains(&needle))
        .map(String::from)
        .collect();
    std::fs::write(index_path, kept.join("\n") + "\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn write_then_read_roundtrips() {
        let dir = tempdir().unwrap();
        let store = LocalMarkdownMemoryStore::new(dir.path().to_path_buf()).unwrap();

        let mem = Memory {
            id: "user_lang".into(),
            name: "Prefers 繁中".into(),
            description: "User writes in Traditional Chinese".into(),
            memory_type: MemoryType::Preference,
            created: Utc::now(),
            last_used: Utc::now(),
            body: "Always reply in 繁體中文.".into(),
        };

        store.write(mem.clone()).await.unwrap();

        let read = store.read("user_lang").await.unwrap().unwrap();
        assert_eq!(read.name, "Prefers 繁中");
        assert_eq!(read.body, "Always reply in 繁體中文.");
        assert!(matches!(read.memory_type, MemoryType::Preference));

        let index = store.read_index().await.unwrap();
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].id, "user_lang");
    }

    #[tokio::test]
    async fn search_finds_in_body() {
        let dir = tempdir().unwrap();
        let store = LocalMarkdownMemoryStore::new(dir.path().to_path_buf()).unwrap();
        store
            .write(Memory {
                id: "a".into(),
                name: "A".into(),
                description: "".into(),
                memory_type: MemoryType::Other("note".into()),
                created: Utc::now(),
                last_used: Utc::now(),
                body: "Coffee at the forest cafe.".into(),
            })
            .await
            .unwrap();
        store
            .write(Memory {
                id: "b".into(),
                name: "B".into(),
                description: "".into(),
                memory_type: MemoryType::Other("note".into()),
                created: Utc::now(),
                last_used: Utc::now(),
                body: "Buy groceries.".into(),
            })
            .await
            .unwrap();

        let hits = store.search("forest", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "a");
    }
}
