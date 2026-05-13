# Memory System

Mori 的記憶機制設計。三層分離 + Markdown 為底 + Tool-First。

## 設計原則

1. **Markdown 是儲存單位,不是 vector DB** — LLM 對檔案系統熟,可審計、可同步、效能不夠再加速
2. **跟 Claude Code auto-memory 同款結構** — 已驗證有用、Letta 研究背書
3. **三層分離對應不同生命週期** — Core(常駐)/ Working(會話)/ Archival(長期)
4. **Cross-device 從 day 1 預留** — phase 7+ 加 SyncedMemoryStore 不重寫

## 三層

### Core Memory(永遠在 LLM context)

```
~/.mori/MEMORY.md
```

格式:索引 + 短描述,~2KB 上限。每次對話都灌進 system prompt。

```markdown
- [User identity](user_identity.md) — 林亞澤,繁中為主,軟體開發者
- [Active task](active_task.md) — 在做 mori-desktop phase 1
- [Recent skills](recent_skills.md) — 上次 EchoSkill 結果
```

對應 Claude Code 的 `MEMORY.md` 索引檔。Mori 寫的時候會自己維護這個檔的精簡。

### Working Memory(本次 session)

```
~/.mori/sessions/<timestamp>/
├── conversation.jsonl       逐輪對話 (role + content + timestamp)
├── skill_calls.jsonl        每次 tool call + 結果
└── context_snapshots/        每次抓的 Context bundle
```

Session 結束(關閉 Mori 或長時間閒置)→ LLM 對 session 寫摘要 → 摘要進 archival,raw session 保留 N 天後刪除。

### Archival Memory(長期可搜尋)

```
~/.mori/memory/                        ← 跟 Claude Code auto-memory 同款
├── MEMORY.md                          ← 全索引(每行一個 memory)
├── user_preferences.md                ← frontmatter: type=preference
├── skill_history_translate.md         ← frontmatter: type=skill_outcome
├── project_mori_desktop.md            ← frontmatter: type=project
├── reference_groq_quirks.md           ← frontmatter: type=reference
└── ...
```

每個 memory 檔的格式:

```markdown
---
name: User prefers terse responses
description: Established 2026-05-06; user explicitly asked for shorter style
type: preference
created: 2026-05-06T14:23
last_used: 2026-05-07T09:15
---

User prefers responses without:
- Unnecessary preamble
- Repeating what they just said
- Step-by-step explanations they didn't ask for
```

搜尋方式(由簡入繁,看效果決定):

| Phase | 機制 |
|---|---|
| 1E | **目前實作**:system prompt 只送索引(`read_index_as_context`),LLM 看到後自行判斷需要哪幾筆 → 呼叫 `recall_memory(id)` skill 拉 body(multi-turn tool call)|
| 5+ | 加 `sqlite-vec` embedding 加速,大量記憶時 LLM 透過 `search_memory(query)` skill 走語意搜尋 |
| 7+ | 跨裝置 CRDT 合併 |
| 9+ | 自動分類 / 自動失效(`stale` 標記) |

## 對應的 Trait

```rust
// crates/mori-core/src/memory/mod.rs

pub struct Memory {
    pub id: String,             // 檔名(不含 .md)
    pub name: String,
    pub description: String,
    pub memory_type: MemoryType,
    pub created: DateTime<Utc>,
    pub last_used: DateTime<Utc>,
    pub body: String,           // markdown content
}

pub enum MemoryType {
    UserIdentity,
    Preference,
    SkillOutcome,
    Project,
    Reference,
    VoiceDict,         // 5E-3: VoiceInput cleanup 校正詞庫
    Other(String),
}

#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// 讀完整索引(MEMORY.md)
    async fn read_index(&self) -> Result<Vec<MemoryIndexEntry>>;

    /// 讀單一 memory
    async fn read(&self, id: &str) -> Result<Option<Memory>>;

    /// 寫入 / 更新
    async fn write(&self, memory: Memory) -> Result<()>;

    /// 搜尋(phase 1-4: grep + LLM 判斷;phase 5+: vec)
    async fn search(&self, query: &str, limit: usize) -> Result<Vec<Memory>>;

    /// 刪除
    async fn delete(&self, id: &str) -> Result<()>;

    /// 訂閱事件流(寫入 / 更新 / 刪除)
    fn observe(&self) -> BoxStream<'static, MemoryEvent>;

    /// 5E-3: 按 type 列出 memory(逐檔讀 frontmatter 過濾)
    async fn list_by_types(&self, types: &[MemoryType]) -> Result<Vec<Memory>>;
}
```

## Memory type 一覽

| type | 寫入 | 讀取 | 用途 |
|---|---|---|---|
| `user_identity` | Agent | Core index | 「我是誰」`遲名 / 角色 / 偏好風格` |
| `preference` | Agent | Core index | 「我喜歡」`簡潔回應 / 不要 emoji` 之類個人偏好 |
| `skill_outcome` | Agent(自動) | Core index | Skill 執行結果(translate / polish 後續可 recall) |
| `project` | Agent | Core index | 進行中的專案脈絡 |
| `reference` | Agent | Core index | 外部 link / Slack channel / Linear project 等 |
| **`voice_dict`** | Agent | **VoiceInput inject_memory_types** | **5E-3:校正詞庫** — Whisper 容易翻錯的人名 / 公司名 / 專有名詞 / 個人慣用語。Voice profile 設 `inject_memory_types: [voice_dict]` 後拼進 cleanup LLM system prompt |
| `Other(X)` | Agent | 自訂用途 | 你自己想分 type 都行 |

### voice_dict 範例

```markdown
---
name: STT 校正詞庫
description: 人名 / 公司名 / 專有名詞 Whisper 容易翻錯的清單
type: voice_dict
created: 2026-05-12T22:00:00Z
last_used: 2026-05-12T22:00:00Z
---

- **Annuli**:Mori 的長期記憶 repo 名,Whisper 容易翻成「安奴利」/「安列利」
- **world-tree**:不要翻成「世界樹」,英文保留
- **智凱**:同事人名,Whisper 常翻成「植凱」/「致凱」
- 「也就是」是我慣用語,不要被改成「就是」
```

要啟用:
1. **Agent 模式**(`Ctrl+Alt+Space`)講「幫我記一個 voice_dict:Annuli 不要翻成安奴利」→ Mori `remember` skill 寫到 `~/.mori/memory/`
2. **Voice profile** 加 `inject_memory_types: [voice_dict]`(或全域 `config.json` `voice_input.inject_memory_types: ["voice_dict"]`)
3. 下次 `Ctrl+Alt+Space` 講話時 cleanup LLM 會看到這份詞庫,校正時參考

## Phase 1 實作:`LocalMarkdownMemoryStore`

```rust
// crates/mori-core/src/memory/markdown.rs

pub struct LocalMarkdownMemoryStore {
    root: PathBuf,                        // ~/.mori/memory/
}

impl LocalMarkdownMemoryStore {
    pub fn new(root: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&root)?;
        let index = root.join("MEMORY.md");
        if !index.exists() {
            std::fs::write(&index, "# Mori Memory Index\n\n")?;
        }
        Ok(Self { root })
    }
}
```

目前實作:`LocalMarkdownMemoryStore`(`crates/mori-core/src/memory/markdown.rs`)
全套 read / write / search / delete / observe 都已落地。grep + LLM 判斷的搜尋
路徑已 ship,vector embedding 等下階段才上(見 phase 表)。

## 跨裝置(Phase 7+ 願景)

```rust
// crates/mori-core/src/memory/synced.rs

pub struct SyncedMemoryStore<L: MemoryStore, R: MemoryStore> {
    local: L,
    remote: R,
    crdt: Arc<RwLock<yrs::Doc>>,
}
```

每個 memory 檔的 frontmatter 加:
```yaml
last_modified: 2026-05-07T14:23:00Z
device_id: acer-sf14-73
content_hash: sha256:...
```

衝突解決:雙方都改了同一個檔 → CRDT 自動合併;若 CRDT 也合不出來(語意衝突)→ 拉 LLM 看兩版,merge 成第三版。

## 跟 Annuli 的關係(Phase 9+)

Annuli 已有更成熟的記憶結構(persona / users / rings / knowledge / drafts)。等 Annuli 加上 MCP server 後,Mori 多一個實作:

```rust
pub struct AnnuliMcpMemoryStore {
    mcp_client: McpClient,
}
```

把 `read` / `write` / `search` 都轉成 MCP tool calls 給 Annuli 處理。Mori 變成 Annuli 的客戶端,共用同一個靈魂。

目前不接 Annuli,先用 LocalMarkdown 建立信心,等 Annuli 那邊穩定再切換。

## 跟 Claude Code auto-memory 的關係

刻意對齊。理由:

1. **Claude Code auto-memory 已驗證有用**(這個 session 就在用)
2. **可 portable**:你的 Claude Code memory 跟 Mori memory 用同款結構,理論上可互通
3. **Letta 研究結論**:filesystem-based 記憶不輸專門框架

未來可考慮:Mori 啟動時自動讀 Claude Code 的 `MEMORY.md` 當 seed(若有),反之亦然。前提是兩邊命名規約對齊。
