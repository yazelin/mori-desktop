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
//! Phase 1 只先做 trait 骨架,實際 read/write 在後續 PR 補上。

use anyhow::Result;
use async_trait::async_trait;
use futures_util::stream::{self, BoxStream};
use std::path::PathBuf;

use super::{Memory, MemoryEvent, MemoryIndexEntry, MemoryStore};

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
}

#[async_trait]
impl MemoryStore for LocalMarkdownMemoryStore {
    async fn read_index(&self) -> Result<Vec<MemoryIndexEntry>> {
        // TODO(phase 1B): parse MEMORY.md → Vec<MemoryIndexEntry>
        Ok(Vec::new())
    }

    async fn read(&self, _id: &str) -> Result<Option<Memory>> {
        // TODO(phase 1B): read <id>.md, parse frontmatter + body
        Ok(None)
    }

    async fn write(&self, _memory: Memory) -> Result<()> {
        // TODO(phase 1B): write <id>.md with YAML frontmatter,
        //                 update MEMORY.md index entry
        Ok(())
    }

    async fn search(&self, _query: &str, _limit: usize) -> Result<Vec<Memory>> {
        // TODO(phase 1B): grep across files, return matching Memory
        // TODO(phase 5):  add sqlite-vec backed implementation
        Ok(Vec::new())
    }

    async fn delete(&self, _id: &str) -> Result<()> {
        // TODO(phase 1B)
        Ok(())
    }

    fn observe(&self) -> BoxStream<'static, MemoryEvent> {
        // TODO(phase 1B): broadcast events on write/update/delete
        Box::pin(stream::empty())
    }
}
