// 5L-3: Profile 編輯器(從 ProfilesTab.tsx 拆出來)。
//
// Modal 內三個 view:
// - Form    typed 表單(frontmatter common fields)+ body textarea
// - Skills  shell_skills 表格編輯器(僅 agent profile)
// - Raw     完整 .md textarea(壞掉 / 進階時用)
//
// 共用一個內部 state(frontmatter object + body string),三個 view 都讀寫同一份。
// 切 view 時自動 round-trip;Raw view 可能 YAML parse 失敗,UI 提示但仍允許存

import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { splitFrontmatter, buildProfileText, AnyObj } from "./profile-form";
import { IconVoiceMic, IconTree, IconClose, IconWarning, IconCheck } from "./icons";
import { Select } from "./Select";

type Kind = "voice" | "agent";

type SaveStatus =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "ok" }
  | { kind: "err"; message: string };

type View = "form" | "skills" | "raw";

const ALL_PROVIDERS = [
  "groq", "gemini", "ollama",
  "claude-cli", "claude-bash",
  "gemini-bash", "codex-bash", "gemini-cli", "codex-cli",
];

const STT_PROVIDERS = ["groq", "whisper-local"];

const BUILT_IN_SKILLS = [
  "translate", "polish", "summarize", "compose",
  "remember", "recall_memory", "forget_memory", "edit_memory",
  "open_url", "open_app", "send_keys", "google_search",
  "ask_chatgpt", "ask_gemini", "find_youtube", "paste_selection_back",
];

// 5E-3: VoiceInput cleanup 可選 inject 的 memory type — voice profile 內勾選後,
// 對應 ~/.mori/memory/ 那些 type 的 memory 會被拼進 cleanup LLM 的 system prompt。
const INJECTABLE_MEMORY_TYPES = [
  "voice_dict", // 校正詞庫(人名 / 公司名 / 專有名詞)
  "preference",
  "user_identity",
  "project",
  "reference",
  "skill_outcome",
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

// ─── Frontmatter form ──────────────────────────────────────────────

function FrontmatterForm({
  kind,
  fm,
  setFm,
}: {
  kind: Kind;
  fm: AnyObj;
  setFm: (next: AnyObj) => void;
}) {
  const patch = (key: string, value: any) => {
    const next = { ...fm };
    if (value === "" || value == null || (Array.isArray(value) && value.length === 0)) {
      delete next[key];
    } else {
      next[key] = value;
    }
    setFm(next);
  };

  return (
    <div className="mori-frontmatter-form">
      <FormRow label="provider" hint="LLM provider(自訂 OpenAI-compat 端點請於 Config tab 加 providers.<name> 區塊)">
        <Select
          value={fm.provider ?? ""}
          allowEmpty
          emptyLabel="(同 ~/.mori/config.json)"
          onChange={(v) => patch("provider", v)}
          options={(() => {
            // 5N: 若 profile 寫了不在 ALL_PROVIDERS 內的 provider 名(例自訂
            // azure-gpt41),補一條 option 進去讓 Select 顯示得出來,別 fallback 到 placeholder。
            const base = ALL_PROVIDERS.map((p) => ({ value: p, label: p }));
            const cur = fm.provider;
            if (typeof cur === "string" && cur && !ALL_PROVIDERS.includes(cur)) {
              base.push({ value: cur, label: `${cur}(自訂)` });
            }
            return base;
          })()}
        />
      </FormRow>

      <FormRow label="stt_provider" hint="STT provider override(僅 voice 有用)">
        <Select
          value={fm.stt_provider ?? ""}
          allowEmpty
          emptyLabel="(同 config)"
          onChange={(v) => patch("stt_provider", v)}
          options={STT_PROVIDERS.map((p) => ({ value: p, label: p }))}
        />
      </FormRow>

      <FormRow label="enable_read" hint="開了 body 才能用 #file: 引用">
        <input
          type="checkbox"
          checked={!!fm.enable_read}
          onChange={(e) => patch("enable_read", e.target.checked || null)}
        />
      </FormRow>

      {kind === "voice" && (
        <>
          <FormRow label="paste_shortcut" hint="貼回游標時用的快捷鍵">
            <Select
              value={fm.paste_shortcut ?? ""}
              allowEmpty
              emptyLabel="(自動偵測)"
              onChange={(v) => patch("paste_shortcut", v)}
              options={[
                { value: "ctrl_v", label: "ctrl_v(一般 app)" },
                { value: "ctrl_shift_v", label: "ctrl_shift_v(terminal)" },
              ]}
            />
          </FormRow>

          <FormRow label="cleanup_level" hint="覆蓋全域 cleanup_level">
            <Select
              value={fm.cleanup_level ?? ""}
              allowEmpty
              emptyLabel="(用 config 預設)"
              onChange={(v) => patch("cleanup_level", v)}
              options={[
                { value: "smart", label: "smart" },
                { value: "minimal", label: "minimal" },
                { value: "none", label: "none" },
              ]}
            />
          </FormRow>

          <FormRow label="enable_auto_enter" hint="貼完後模擬 Enter">
            <input
              type="checkbox"
              checked={!!fm.enable_auto_enter}
              onChange={(e) => patch("enable_auto_enter", e.target.checked || null)}
            />
          </FormRow>

          <FormRow
            label="inject_memory_types"
            hint="cleanup LLM 注入 ~/.mori/memory 內這些 type 的 memory(空 = 走 config 預設;勾任一 = profile 自己決定)"
          >
            <MemoryTypeChipsEditor
              value={Array.isArray(fm.inject_memory_types) ? fm.inject_memory_types : []}
              onChange={(v) => patch("inject_memory_types", v)}
            />
          </FormRow>
        </>
      )}

      {kind === "agent" && (
        <FormRow label="enabled_skills" hint="這個 profile 啟用的 built-in skills,留空 = 全開">
          <SkillChipsEditor
            value={Array.isArray(fm.enabled_skills) ? fm.enabled_skills : []}
            onChange={(v) => patch("enabled_skills", v)}
          />
        </FormRow>
      )}
    </div>
  );
}

// ─── Memory type chips (5E-3: VoiceInput inject_memory_types)──────

function MemoryTypeChipsEditor({
  value,
  onChange,
}: {
  value: string[];
  onChange: (next: string[]) => void;
}) {
  const toggle = (t: string) => {
    if (value.includes(t)) onChange(value.filter((x) => x !== t));
    else onChange([...value, t]);
  };
  return (
    <div className="mori-skill-chips">
      {INJECTABLE_MEMORY_TYPES.map((t) => (
        <button
          key={t}
          className={`mori-skill-chip ${value.includes(t) ? "on" : ""}`}
          onClick={() => toggle(t)}
          type="button"
        >
          {value.includes(t) ? "✓ " : ""}{t}
        </button>
      ))}
    </div>
  );
}

// ─── Skill chips (multi-select) ────────────────────────────────────

function SkillChipsEditor({
  value,
  onChange,
}: {
  value: string[];
  onChange: (next: string[]) => void;
}) {
  const toggle = (s: string) => {
    if (value.includes(s)) onChange(value.filter((x) => x !== s));
    else onChange([...value, s]);
  };
  return (
    <div className="mori-skill-chips">
      {BUILT_IN_SKILLS.map((s) => (
        <button
          key={s}
          className={`mori-skill-chip ${value.includes(s) ? "on" : ""}`}
          onClick={() => toggle(s)}
          type="button"
        >
          {value.includes(s) ? "✓ " : ""}{s}
        </button>
      ))}
    </div>
  );
}

// ─── shell_skills editor ───────────────────────────────────────────

type ShellSkillDef = {
  name: string;
  description: string;
  command: string[] | string;
  parameters?: Record<string, { type?: string; required?: boolean; description?: string; default?: any }>;
  timeout?: number;
  working_dir?: string;
  success_message?: string;
};

function ShellSkillsEditor({
  skills,
  setSkills,
}: {
  skills: ShellSkillDef[];
  setSkills: (next: ShellSkillDef[]) => void;
}) {
  const update = (i: number, patch: Partial<ShellSkillDef>) => {
    const next = [...skills];
    next[i] = { ...next[i], ...patch };
    setSkills(next);
  };
  const remove = (i: number) => setSkills(skills.filter((_, j) => j !== i));
  const add = () => setSkills([
    ...skills,
    {
      name: "",
      description: "",
      command: ["echo", "{{message}}"],
    },
  ]);

  return (
    <div className="mori-shell-skills">
      {skills.length === 0 && (
        <div className="mori-shell-skills-empty">
          目前沒有 shell_skill。按下面加一個 — 例如 gh PR list / docker ps / ssh 到指定主機。
        </div>
      )}
      {skills.map((s, i) => (
        <ShellSkillCard
          key={i}
          skill={s}
          onPatch={(patch) => update(i, patch)}
          onRemove={() => remove(i)}
        />
      ))}
      <button className="mori-btn" onClick={add}>+ 新增 shell_skill</button>
    </div>
  );
}

// 5L-5: 從 command 抽 {{name}} placeholder,跟 parameters 字典 cross-check
function extractPlaceholders(command: string[] | string): string[] {
  const text = Array.isArray(command) ? command.join(" ") : command;
  const re = /\{\{\s*([A-Za-z_][A-Za-z0-9_]*)\s*\}\}/g;
  const found = new Set<string>();
  let m: RegExpExecArray | null;
  while ((m = re.exec(text))) found.add(m[1]);
  return Array.from(found);
}

function ShellSkillCard({
  skill,
  onPatch,
  onRemove,
}: {
  skill: ShellSkillDef;
  onPatch: (patch: Partial<ShellSkillDef>) => void;
  onRemove: () => void;
}) {
  const [collapsed, setCollapsed] = useState(false);
  const commandStr = Array.isArray(skill.command)
    ? skill.command.map((c) => JSON.stringify(c)).join(" ")
    : skill.command;

  // 5L-5: placeholders vs parameters cross-check
  const usedPlaceholders = extractPlaceholders(skill.command);
  const declaredParams = Object.keys(skill.parameters ?? {});
  const usedNotDeclared = usedPlaceholders.filter((p) => !declaredParams.includes(p));
  const declaredNotUsed = declaredParams.filter((p) => !usedPlaceholders.includes(p));
  const setCommand = (text: string) => {
    // 嘗試 JSON-array 解析(["foo","bar baz"]),失敗就 split by space(僅最簡 case)
    let parsed: string[] | string | null = null;
    try {
      const j = JSON.parse(text);
      if (Array.isArray(j)) parsed = j.map(String);
    } catch {}
    if (parsed === null) {
      // 簡單 tokenize: shlex-lite,接受 "..." / '...' / bare word
      parsed = tokenize(text);
    }
    onPatch({ command: parsed });
  };

  const params = Object.entries(skill.parameters ?? {});
  const updateParam = (key: string, field: string, value: any) => {
    const params: AnyObj = { ...(skill.parameters ?? {}) };
    params[key] = { ...(params[key] ?? {}), [field]: value };
    if (value === "" || value == null) delete params[key][field];
    if (Object.keys(params[key]).length === 0) delete params[key];
    onPatch({ parameters: Object.keys(params).length === 0 ? undefined : params });
  };
  const renameParam = (oldKey: string, newKey: string) => {
    if (!newKey || newKey === oldKey) return;
    const params: AnyObj = { ...(skill.parameters ?? {}) };
    params[newKey] = params[oldKey];
    delete params[oldKey];
    onPatch({ parameters: params });
  };
  const removeParam = (key: string) => {
    const params: AnyObj = { ...(skill.parameters ?? {}) };
    delete params[key];
    onPatch({ parameters: Object.keys(params).length === 0 ? undefined : params });
  };
  const addParam = () => {
    const params: AnyObj = { ...(skill.parameters ?? {}) };
    let i = 1;
    while (params[`param${i}`]) i++;
    params[`param${i}`] = { type: "string", required: false };
    onPatch({ parameters: params });
  };

  return (
    <div className={`mori-shell-skill-card ${collapsed ? "collapsed" : ""}`}>
      <div className="mori-shell-skill-head">
        <input
          className="mori-input"
          placeholder="name(例 gh_pr_list)"
          value={skill.name}
          onChange={(e) => onPatch({ name: e.target.value })}
        />
        <button className="mori-btn small ghost" onClick={() => setCollapsed(!collapsed)}>
          {collapsed ? "▸" : "▾"}
        </button>
        <button className="mori-btn small danger" onClick={onRemove}>刪除</button>
      </div>
      {!collapsed && (
        <div className="mori-shell-skill-body">
          <FormRow label="description" hint="LLM 看到的說明">
            <textarea
              className="mori-input"
              rows={2}
              value={skill.description}
              onChange={(e) => onPatch({ description: e.target.value })}
            />
          </FormRow>
          <FormRow label="command" hint='JSON array["bin","arg",...] 或 shell-like 字串'>
            <div>
              <textarea
                className="mori-input mono"
                rows={2}
                value={commandStr}
                onChange={(e) => setCommand(e.target.value)}
                placeholder='["gh", "pr", "list", "--repo", "{{repo}}"]'
              />
              {/* 5L-5: placeholder ↔ parameters 一致性檢查 */}
              {(usedNotDeclared.length > 0 || declaredNotUsed.length > 0) && (
                <div className="mori-placeholder-check">
                  {usedNotDeclared.length > 0 && (
                    <div className="warn">
                      <IconWarning width={12} height={12} /> command 用到但 parameters 沒宣告:
                      {usedNotDeclared.map((p) => (
                        <button
                          key={p}
                          className="mori-placeholder-chip add"
                          onClick={() => {
                            const next = { ...(skill.parameters ?? {}) };
                            next[p] = { type: "string", required: true };
                            onPatch({ parameters: next });
                          }}
                          title={`加為 string required 參數`}
                        >
                          {`{{${p}}}`} <span className="action">+ 加</span>
                        </button>
                      ))}
                    </div>
                  )}
                  {declaredNotUsed.length > 0 && (
                    <div className="hint">
                      <span style={{ opacity: 0.7 }}>i</span> parameters 宣告了但 command 沒用:
                      {declaredNotUsed.map((p) => (
                        <span key={p} className="mori-placeholder-chip dim">{p}</span>
                      ))}
                    </div>
                  )}
                </div>
              )}
              {usedPlaceholders.length > 0
                && usedNotDeclared.length === 0
                && declaredNotUsed.length === 0 && (
                <div className="mori-placeholder-check ok">
                  <IconCheck width={12} height={12} /> {usedPlaceholders.length} 個 placeholder 都對應到 parameters
                </div>
              )}
            </div>
          </FormRow>
          <FormRow label="timeout" hint="秒,空 = 30">
            <input
              className="mori-input"
              type="number"
              value={skill.timeout ?? ""}
              onChange={(e) => onPatch({ timeout: e.target.value === "" ? undefined : Number(e.target.value) })}
            />
          </FormRow>
          <FormRow label="working_dir" hint="可空(用 mori-tauri cwd)">
            <input
              className="mori-input"
              value={skill.working_dir ?? ""}
              onChange={(e) => onPatch({ working_dir: e.target.value || undefined })}
            />
          </FormRow>
          <FormRow label="success_message" hint="完成時的 chat 提示(可含 {{變數}})">
            <input
              className="mori-input"
              value={skill.success_message ?? ""}
              onChange={(e) => onPatch({ success_message: e.target.value || undefined })}
            />
          </FormRow>

          <div className="mori-shell-skill-params">
            <div className="mori-shell-skill-params-head">
              <h4>parameters</h4>
              <button className="mori-btn small" onClick={addParam}>+ 加參數</button>
            </div>
            {params.length === 0 && (
              <div className="mori-shell-skills-empty small">沒有參數,command 內 {`{{name}}`} 也無從替換</div>
            )}
            {params.map(([key, def]) => (
              <div key={key} className="mori-shell-param-row">
                <input
                  className="mori-input"
                  placeholder="參數名"
                  value={key}
                  onBlur={(e) => renameParam(key, e.target.value.trim())}
                  onChange={(e) => {
                    // 維持 controlled,但 rename 在 blur 才動
                    e.target.value = e.target.value;
                  }}
                  defaultValue={key}
                />
                <Select
                  value={(def as any).type ?? "string"}
                  onChange={(v) => updateParam(key, "type", v)}
                  options={[
                    { value: "string", label: "string" },
                    { value: "number", label: "number" },
                    { value: "boolean", label: "boolean" },
                  ]}
                />
                <label className="mori-shell-param-required">
                  <input
                    type="checkbox"
                    checked={!!(def as any).required}
                    onChange={(e) => updateParam(key, "required", e.target.checked || undefined)}
                  />
                  required
                </label>
                <input
                  className="mori-input"
                  placeholder="description(可空)"
                  value={(def as any).description ?? ""}
                  onChange={(e) => updateParam(key, "description", e.target.value)}
                />
                <button
                  className="mori-btn small ghost"
                  onClick={() => removeParam(key)}
                  title="刪除參數"
                ><IconClose width={11} height={11} /></button>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

// 最簡 tokenize:支援 "..." / '...' / bare word
function tokenize(text: string): string[] {
  const result: string[] = [];
  let i = 0;
  while (i < text.length) {
    while (i < text.length && /\s/.test(text[i])) i++;
    if (i >= text.length) break;
    if (text[i] === '"' || text[i] === "'") {
      const quote = text[i];
      i++;
      let buf = "";
      while (i < text.length && text[i] !== quote) {
        if (text[i] === "\\" && i + 1 < text.length) { buf += text[i + 1]; i += 2; }
        else { buf += text[i]; i++; }
      }
      i++; // closing quote
      result.push(buf);
    } else {
      let buf = "";
      while (i < text.length && !/\s/.test(text[i])) {
        buf += text[i]; i++;
      }
      result.push(buf);
    }
  }
  return result;
}

// ─── Main editor ──────────────────────────────────────────────────

export function ProfileEditor({
  kind,
  stem,
  onClose,
  onSaved,
}: {
  kind: Kind;
  stem: string;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [raw, setRaw] = useState<string>("");
  const [origRaw, setOrigRaw] = useState<string>("");
  const [loading, setLoading] = useState(true);
  const [view, setView] = useState<View>("form");
  const [status, setStatus] = useState<SaveStatus>({ kind: "idle" });

  // Parse raw into (frontmatter object, body string)
  const split = useMemo(() => splitFrontmatter(raw), [raw]);
  const { frontmatter: fm, body } = split;

  const setFm = (next: AnyObj) => {
    setRaw(buildProfileText(next, body));
  };
  const setBody = (next: string) => {
    setRaw(buildProfileText(fm, next));
  };
  const setShellSkills = (skills: ShellSkillDef[]) => {
    const next = { ...fm };
    if (skills.length === 0) delete next.shell_skills;
    else next.shell_skills = skills;
    setRaw(buildProfileText(next, body));
  };

  useEffect(() => {
    invoke<string>("profile_read", { kind, stem })
      .then((t) => { setRaw(t); setOrigRaw(t); })
      .catch((e) => setStatus({ kind: "err", message: `load: ${e}` }))
      .finally(() => setLoading(false));
  }, [kind, stem]);

  const save = async () => {
    setStatus({ kind: "saving" });
    try {
      await invoke("profile_write", { kind, stem, text: raw });
      setOrigRaw(raw);
      setStatus({ kind: "ok" });
      onSaved();
      setTimeout(() => setStatus({ kind: "idle" }), 2000);
    } catch (e: any) {
      setStatus({ kind: "err", message: String(e) });
    }
  };

  const remove = async () => {
    if (!confirm(`刪除 ${kind}/${stem}.md? 不可復原。`)) return;
    try {
      await invoke("profile_delete", { kind, stem });
      onSaved();
      onClose();
    } catch (e: any) {
      setStatus({ kind: "err", message: String(e) });
    }
  };

  const dirty = raw !== origRaw;

  return (
    <div className="mori-modal-backdrop" onClick={onClose}>
      <div className="mori-modal mori-profile-modal" onClick={(e) => e.stopPropagation()}>
        <div className="mori-modal-header">
          <div className="mori-modal-title">
            <span className="mori-modal-kind">
              {kind === "voice"
                ? <><IconVoiceMic width={12} height={12} /> VoiceInput</>
                : <><IconTree width={12} height={12} /> Agent</>}
            </span>
            <span className="mori-modal-stem">{stem}.md</span>
          </div>
          <div className="mori-view-toggle inline">
            <button
              className={`mori-view-tab ${view === "form" ? "active" : ""}`}
              onClick={() => setView("form")}
            >Form</button>
            {kind === "agent" && (
              <button
                className={`mori-view-tab ${view === "skills" ? "active" : ""}`}
                onClick={() => setView("skills")}
              >Shell Skills</button>
            )}
            <button
              className={`mori-view-tab ${view === "raw" ? "active" : ""}`}
              onClick={() => setView("raw")}
            >Raw</button>
          </div>
          <button className="mori-btn ghost" onClick={onClose} title="關閉"><IconClose width={14} height={14} /></button>
        </div>
        <div className="mori-modal-body">
          {loading ? (
            <div className="mori-modal-loading">讀取中…</div>
          ) : view === "raw" ? (
            <textarea
              className="mori-modal-textarea"
              spellCheck={false}
              value={raw}
              onChange={(e) => setRaw(e.target.value)}
            />
          ) : view === "skills" ? (
            <ShellSkillsEditor
              skills={Array.isArray(fm.shell_skills) ? fm.shell_skills : []}
              setSkills={setShellSkills}
            />
          ) : (
            <div className="mori-profile-form">
              <FrontmatterForm kind={kind} fm={fm} setFm={setFm} />
              <div className="mori-form-divider">
                <span>Body (markdown)</span>
              </div>
              <textarea
                className="mori-modal-textarea body"
                spellCheck={false}
                value={body}
                onChange={(e) => setBody(e.target.value)}
                placeholder="profile body — 人格 / 行為指示 / corrections #file: 等"
              />
            </div>
          )}
        </div>
        <div className="mori-modal-footer">
          <button className="mori-btn danger" onClick={remove}>刪除</button>
          <div className="mori-modal-footer-right">
            {status.kind === "saving" && <span className="mori-save-status saving">儲存中…</span>}
            {status.kind === "ok" && <span className="mori-save-status ok">✓ 已儲存</span>}
            {status.kind === "err" && <span className="mori-save-status err">✗ {status.message}</span>}
            <button className="mori-btn" onClick={() => setRaw(origRaw)} disabled={!dirty}>還原</button>
            <button className="mori-btn primary" onClick={save} disabled={!dirty}>儲存</button>
          </div>
        </div>
      </div>
    </div>
  );
}

export default ProfileEditor;
