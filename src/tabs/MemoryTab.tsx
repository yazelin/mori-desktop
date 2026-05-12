// 5L-4: Memory browser + editor。
// 列 ~/.mori/memory/ 內 .md(skip MEMORY.md 索引),點開可看 / 編 / 刪。

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type MemoryEntry = {
  id: string;
  name: string;
  description: string;
  memory_type: string;
};

type MemoryDetail = MemoryEntry & {
  created: string;
  last_used: string;
  body: string;
};

type SaveStatus =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "ok" }
  | { kind: "err"; message: string };

const TYPE_OPTIONS = [
  "user_identity",
  "preference",
  "skill_outcome",
  "project",
  "reference",
];

function FormRow({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="mori-form-row">
      <div className="mori-form-row-label">
        <span>{label}</span>
        {hint && <span className="mori-form-row-hint">{hint}</span>}
      </div>
      <div className="mori-form-row-input">{children}</div>
    </div>
  );
}

function MemoryEditor({
  id,
  isNew,
  onClose,
  onSaved,
}: {
  id: string;
  isNew: boolean;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [detail, setDetail] = useState<MemoryDetail | null>(null);
  const [loading, setLoading] = useState(!isNew);
  const [status, setStatus] = useState<SaveStatus>({ kind: "idle" });

  // local edit state
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [memoryType, setMemoryType] = useState("preference");
  const [body, setBody] = useState("");

  useEffect(() => {
    if (isNew) {
      setDetail(null);
      return;
    }
    invoke<MemoryDetail | null>("memory_read", { id })
      .then((d) => {
        if (!d) {
          setStatus({ kind: "err", message: "memory not found" });
          return;
        }
        setDetail(d);
        setName(d.name);
        setDescription(d.description);
        setMemoryType(d.memory_type);
        setBody(d.body);
      })
      .catch((e) => setStatus({ kind: "err", message: `load: ${e}` }))
      .finally(() => setLoading(false));
  }, [id, isNew]);

  const save = async () => {
    setStatus({ kind: "saving" });
    try {
      await invoke("memory_write", {
        args: {
          id,
          name: name.trim() || id,
          description: description.trim(),
          memory_type: memoryType,
          body,
        },
      });
      setStatus({ kind: "ok" });
      onSaved();
      setTimeout(() => setStatus({ kind: "idle" }), 1500);
    } catch (e: any) {
      setStatus({ kind: "err", message: String(e) });
    }
  };

  const remove = async () => {
    if (!confirm(`刪除 memory「${name || id}」? 不可復原。`)) return;
    try {
      await invoke("memory_delete", { id });
      onSaved();
      onClose();
    } catch (e: any) {
      setStatus({ kind: "err", message: String(e) });
    }
  };

  return (
    <div className="mori-modal-backdrop" onClick={onClose}>
      <div className="mori-modal mori-memory-modal" onClick={(e) => e.stopPropagation()}>
        <div className="mori-modal-header">
          <div className="mori-modal-title">
            <span className="mori-modal-kind">📓 Memory</span>
            <span className="mori-modal-stem">{id}.md</span>
          </div>
          <button className="mori-btn ghost" onClick={onClose}>✕</button>
        </div>
        <div className="mori-modal-body">
          {loading ? (
            <div className="mori-modal-loading">讀取中…</div>
          ) : (
            <div className="mori-memory-form">
              <FormRow label="name" hint="LLM 看到的標題">
                <input
                  className="mori-input"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                />
              </FormRow>
              <FormRow label="description" hint="一句話描述">
                <input
                  className="mori-input"
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                />
              </FormRow>
              <FormRow label="type">
                <select
                  className="mori-input"
                  value={memoryType}
                  onChange={(e) => setMemoryType(e.target.value)}
                >
                  {TYPE_OPTIONS.map((t) => (
                    <option key={t} value={t}>{t}</option>
                  ))}
                </select>
              </FormRow>
              {detail && (
                <FormRow label="timestamps" hint="自動維護(寫入時更新 last_used)">
                  <div className="mori-memory-timestamps">
                    <span>created: {detail.created}</span>
                    <span>last_used: {detail.last_used}</span>
                  </div>
                </FormRow>
              )}
              <div className="mori-form-divider"><span>Body</span></div>
              <textarea
                className="mori-modal-textarea body"
                spellCheck={false}
                value={body}
                onChange={(e) => setBody(e.target.value)}
                placeholder="記憶內容 — markdown 支援。建議用簡潔的條列 / 重點而非長篇大論。"
              />
            </div>
          )}
        </div>
        <div className="mori-modal-footer">
          {!isNew && (
            <button className="mori-btn danger" onClick={remove}>刪除</button>
          )}
          <div className="mori-modal-footer-right">
            {status.kind === "saving" && <span className="mori-save-status saving">儲存中…</span>}
            {status.kind === "ok" && <span className="mori-save-status ok">✓ 已儲存</span>}
            {status.kind === "err" && <span className="mori-save-status err">✗ {status.message}</span>}
            <button className="mori-btn primary" onClick={save}>儲存</button>
          </div>
        </div>
      </div>
    </div>
  );
}

function MemoryTab() {
  const [entries, setEntries] = useState<MemoryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ id: string; isNew: boolean } | null>(null);
  const [filter, setFilter] = useState("");

  const reload = async () => {
    try {
      const list = await invoke<MemoryEntry[]>("memory_list");
      setEntries(list);
      setError(null);
    } catch (e: any) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { reload(); }, []);

  const createNew = () => {
    const id = prompt("新 memory id(英數 / _ / -,例 user_lang_preference):");
    if (!id) return;
    const trimmed = id.trim();
    if (!trimmed) return;
    if (!/^[A-Za-z0-9_\-.]+$/.test(trimmed)) {
      alert("id 只接受英數 / _ / - / .");
      return;
    }
    setEditing({ id: trimmed, isNew: true });
  };

  const filtered = entries.filter((e) => {
    if (!filter.trim()) return true;
    const q = filter.toLowerCase();
    return (
      e.id.toLowerCase().includes(q) ||
      e.name.toLowerCase().includes(q) ||
      e.description.toLowerCase().includes(q) ||
      e.memory_type.toLowerCase().includes(q)
    );
  });

  return (
    <div className="mori-tab mori-tab-memory">
      <h2 className="mori-tab-title">Memory</h2>
      <p className="mori-tab-hint">
        瀏覽 / 編輯 ~/.mori/memory/ 長期記憶。Mori 自己會用 remember /
        recall_memory / forget_memory / edit_memory skill 維護;你也可以直接編。
        改完即時生效(下一次熱鍵就讀新內容)。
      </p>

      {error && <div className="mori-config-error">{error}</div>}

      <div className="mori-memory-toolbar">
        <input
          className="mori-input"
          placeholder="搜尋 id / name / description..."
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <button className="mori-btn primary" onClick={createNew}>+ 新增記憶</button>
        <button className="mori-btn" onClick={reload}>🔄</button>
        <span className="mori-memory-count">{filtered.length} / {entries.length}</span>
      </div>

      {loading ? (
        <div className="mori-tab-placeholder"><p>讀取中…</p></div>
      ) : entries.length === 0 ? (
        <div className="mori-tab-placeholder">
          <p>目前沒有任何 memory。</p>
          <p>跟 Mori 對話時請他「記住...」會自動寫,或按上面「+ 新增記憶」手動加。</p>
        </div>
      ) : (
        <div className="mori-memory-list">
          {filtered.map((m) => (
            <div key={m.id} className="mori-memory-row" onClick={() => setEditing({ id: m.id, isNew: false })}>
              <div className="mori-memory-row-main">
                <span className="mori-memory-name">{m.name || m.id}</span>
                <span className={`mori-memory-type type-${m.memory_type}`}>{m.memory_type}</span>
              </div>
              <div className="mori-memory-row-sub">
                <span className="mori-memory-desc">{m.description || <em>(無描述)</em>}</span>
                <span className="mori-memory-id">{m.id}</span>
              </div>
            </div>
          ))}
          {filtered.length === 0 && (
            <div className="mori-tab-placeholder"><p>沒有符合篩選的 memory</p></div>
          )}
        </div>
      )}

      {editing && (
        <MemoryEditor
          id={editing.id}
          isNew={editing.isNew}
          onClose={() => setEditing(null)}
          onSaved={reload}
        />
      )}
    </div>
  );
}

export default MemoryTab;
