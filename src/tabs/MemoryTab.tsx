// 5L-4: Memory browser + editor。
// 列 ~/.mori/memory/ 內 .md(skip MEMORY.md 索引),點開可看 / 編 / 刪。

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { IconMemory, IconClose, IconRefresh } from "../icons";
import { Select } from "../Select";

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
  "voice_dict", // 5E-3:VoiceInput cleanup 校正詞庫 / 專有名詞 / 個人慣用語
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
  const { t } = useTranslation();
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
    if (!confirm(t("memory_tab.confirm_delete", { name: name || id }))) return;
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
            <span className="mori-modal-kind"><IconMemory width={12} height={12} /> Memory</span>
            <span className="mori-modal-stem">{id}.md</span>
          </div>
          <button className="mori-btn ghost" onClick={onClose} title={t("memory_tab.close_title")}><IconClose width={14} height={14} /></button>
        </div>
        <div className="mori-modal-body">
          {loading ? (
            <div className="mori-modal-loading">{t("memory_tab.reading")}</div>
          ) : (
            <div className="mori-memory-form">
              <FormRow label="name" hint={t("memory_tab.label_name_hint")}>
                <input
                  className="mori-input"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                />
              </FormRow>
              <FormRow label="description" hint={t("memory_tab.label_desc_hint")}>
                <input
                  className="mori-input"
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                />
              </FormRow>
              <FormRow label="type">
                <Select
                  value={memoryType}
                  onChange={setMemoryType}
                  options={TYPE_OPTIONS.map((opt) => ({ value: opt, label: opt }))}
                />
              </FormRow>
              {detail && (
                <FormRow label="timestamps" hint={t("memory_tab.label_timestamps_hint")}>
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
                placeholder={t("memory_tab.body_placeholder")}
              />
            </div>
          )}
        </div>
        <div className="mori-modal-footer">
          {!isNew && (
            <button className="mori-btn danger" onClick={remove}>{t("memory_tab.delete_button")}</button>
          )}
          <div className="mori-modal-footer-right">
            {status.kind === "saving" && <span className="mori-save-status saving">{t("memory_tab.saving")}</span>}
            {status.kind === "ok" && <span className="mori-save-status ok">{t("memory_tab.saved")}</span>}
            {status.kind === "err" && <span className="mori-save-status err">✗ {status.message}</span>}
            <button className="mori-btn primary" onClick={save}>{t("common.save")}</button>
          </div>
        </div>
      </div>
    </div>
  );
}

function MemoryTab() {
  const { t } = useTranslation();
  const [entries, setEntries] = useState<MemoryEntry[]>([]);
  const [hits, setHits] = useState<MemoryEntry[] | null>(null); // null = 沒在搜尋
  const [searching, setSearching] = useState(false);
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

  // 5L-5: 搜尋 debounce 300ms,空 query 回 list-only。
  // 非空走 memory_search(會掃 body),空就清空 hits。
  useEffect(() => {
    const q = filter.trim();
    if (!q) {
      setHits(null);
      setSearching(false);
      return;
    }
    setSearching(true);
    const handle = setTimeout(async () => {
      try {
        const res = await invoke<MemoryEntry[]>("memory_search", { query: q, limit: 100 });
        setHits(res);
      } catch (e: any) {
        setError(String(e));
      } finally {
        setSearching(false);
      }
    }, 300);
    return () => clearTimeout(handle);
  }, [filter]);

  const createNew = () => {
    const id = prompt(t("memory_tab.new_id_prompt"));
    if (!id) return;
    const trimmed = id.trim();
    if (!trimmed) return;
    if (!/^[A-Za-z0-9_\-.]+$/.test(trimmed)) {
      alert(t("memory_tab.invalid_id"));
      return;
    }
    setEditing({ id: trimmed, isNew: true });
  };

  // 有 query → 用後端 hits(含 body 搜尋);無 query → 用 list
  const filtered = hits ?? entries;

  return (
    <div className="mori-tab mori-tab-memory">
      <h2 className="mori-tab-title">Memory</h2>
      <p className="mori-tab-hint">{t("memory_tab.hint")}</p>

      {error && <div className="mori-config-error">{error}</div>}

      <div className="mori-memory-toolbar">
        <input
          className="mori-input"
          placeholder={t("memory_tab.search_placeholder")}
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <button className="mori-btn primary" onClick={createNew}>{t("memory_tab.new_button")}</button>
        <button className="mori-btn" onClick={reload} title={t("memory_tab.refresh_title")}><IconRefresh width={13} height={13} /></button>
        <span className="mori-memory-count">
          {searching ? t("memory_tab.searching") : `${filtered.length} / ${entries.length}`}
        </span>
      </div>

      {loading ? (
        <div className="mori-tab-placeholder"><p>{t("memory_tab.loading")}</p></div>
      ) : entries.length === 0 ? (
        <div className="mori-tab-placeholder">
          <p>{t("memory_tab.empty_title")}</p>
          <p>{t("memory_tab.empty_hint")}</p>
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
                <span className="mori-memory-desc">{m.description || <em>{t("memory_tab.no_description")}</em>}</span>
                <span className="mori-memory-id">{m.id}</span>
              </div>
            </div>
          ))}
          {filtered.length === 0 && (
            <div className="mori-tab-placeholder"><p>{t("memory_tab.no_match")}</p></div>
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
