// 5L-2: ~/.mori/config.json typed form + raw JSON 雙模式編輯器。
//
// 設計:
// - 預設 Form view:常用欄位 typed inputs / dropdowns
// - Raw JSON view:textarea + 即時 parse 驗證,給 power user 加 routing.skills
//   等進階欄位
// - 兩個 view 共用一個 JSON state source-of-truth;切換時自動 sync,
//   未列在 form 的 key 也會保留(round-trip 不丟資料)
// - 儲存:寫整個 JSON 物件

import { useEffect, useMemo, useState, type SVGProps } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { listThemes, setActiveTheme, themesDir, loadActiveTheme, type ThemeEntry } from "../theme";
import { Select } from "../Select";
import {
  IconHome,
  IconCloud,
  IconVoiceMic,
  IconTree,
  IconKeyboard,
  IconClipboard,
  IconPencil,
} from "../icons";

// 5P-6: character pack picker
type CharacterEntry = {
  stem: string;
  display_name: string;
  author: string;
  version: string;
};

type SaveStatus =
  | { kind: "idle" }
  | { kind: "saving" }
  | { kind: "ok" }
  | { kind: "err"; message: string };

// 已知的 named providers — 來自 mori-core/src/llm/mod.rs
const ALL_PROVIDERS = [
  "groq",
  "gemini",
  "ollama",
  "claude-cli",
  "claude-bash",
  "gemini-bash",
  "codex-bash",
  "gemini-cli",
  "codex-cli",
] as const;

const STT_PROVIDERS = ["groq", "whisper-local"] as const;

function Section({
  title,
  hint,
  children,
}: {
  title: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="mori-config-section">
      <h3 className="mori-config-section-title">{title}</h3>
      {hint && <p className="mori-config-section-hint">{hint}</p>}
      {children}
    </section>
  );
}

function StatusBadge({ status }: { status: SaveStatus }) {
  if (status.kind === "idle") return null;
  if (status.kind === "saving") return <span className="mori-save-status saving">儲存中…</span>;
  if (status.kind === "ok") return <span className="mori-save-status ok">✓ 已儲存</span>;
  return <span className="mori-save-status err">✗ {status.message}</span>;
}

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
        {hint && <HintTooltip>{hint}</HintTooltip>}
      </div>
      <div className="mori-form-row-input">{children}</div>
    </div>
  );
}

/** ⓘ icon + hover/focus 後出 popover 顯示 hint。把長 hint 從 inline 文字
 *  改成 on-demand 提示,大幅減少 Config tab 垂直密度。 */
function HintTooltip({ children }: { children: React.ReactNode }) {
  return (
    <span className="mori-hint" tabIndex={0} aria-label="說明">
      <span className="mori-hint-icon">ⓘ</span>
      <span className="mori-hint-popover">{children}</span>
    </span>
  );
}

type SubTabId =
  | "quick"
  | "llm"
  | "voice"
  | "appearance"
  | "hotkey"
  | "x11"
  | "corrections"
  | "raw";

interface SubTabSpec {
  id: SubTabId;
  label: string;
  Icon: React.ComponentType<SVGProps<SVGSVGElement>>;
}

/** Config tab 左側垂直 sub-nav。x11 sub-tab 由 caller 決定是否傳入。 */
function SubTabNav({
  active,
  onChange,
  tabs,
  dirtyJson,
}: {
  active: SubTabId;
  onChange: (id: SubTabId) => void;
  tabs: SubTabSpec[];
  dirtyJson: boolean;
}) {
  return (
    <nav className="mori-config-subnav">
      {tabs.map((t) => {
        const Icon = t.Icon;
        return (
          <button
            key={t.id}
            type="button"
            className={`mori-config-subtab ${active === t.id ? "active" : ""}`}
            onClick={() => onChange(t.id)}
          >
            <span className="mori-config-subtab-icon">
              <Icon width={14} height={14} />
            </span>
            <span className="mori-config-subtab-label">{t.label}</span>
            {dirtyJson && t.id === "raw" && (
              <span className="mori-config-subtab-dirty" title="JSON 有未存改動" />
            )}
          </button>
        );
      })}
    </nav>
  );
}

function KvTable({
  rows,
  setRows,
  keyPlaceholder,
  valuePlaceholder,
  valueIsSecret = false,
}: {
  rows: Array<{ k: string; v: string }>;
  setRows: (rows: Array<{ k: string; v: string }>) => void;
  keyPlaceholder?: string;
  valuePlaceholder?: string;
  valueIsSecret?: boolean;
}) {
  // 之前用 props.rows 直接驅動 render — 但 parent 的 setRows callback 會把
  // 空 key 的 row 過濾掉(不能寫進 JSON object),導致按「+ 新增」加的空白
  // row 在下一個 render 就被父層 filter 掉、看起來像按鈕沒反應。
  //
  // 改用 internal state:KvTable 保有「正在編輯的」rows(含空白草稿),
  // 父層只收到非空 key 的 rows 寫進 cfg。第一次掛載從 props 初始化;之後
  // 不再 sync from prop(父層 cfg 改也不會把使用者打到一半的草稿沖掉)。
  const [localRows, setLocalRows] = useState<Array<{ k: string; v: string }>>(rows);
  const update = (i: number, field: "k" | "v", value: string) => {
    const next = [...localRows];
    next[i] = { ...next[i], [field]: value };
    apply(next);
  };
  const remove = (i: number) => apply(localRows.filter((_, j) => j !== i));
  const add = () => apply([...localRows, { k: "", v: "" }]);
  // 寫回父層只給有 key 的 row;空白 key 是「正在 typing」的草稿,留在本地。
  const apply = (next: Array<{ k: string; v: string }>) => {
    setLocalRows(next);
    setRows(next.filter((r) => r.k.trim()));
  };
  return (
    <div className="mori-kv-table">
      {localRows.map((r, i) => (
        <div key={i} className="mori-kv-row">
          <input
            className="mori-kv-key"
            value={r.k}
            onChange={(e) => update(i, "k", e.target.value)}
            placeholder={keyPlaceholder}
          />
          <input
            className="mori-kv-value"
            type={valueIsSecret ? "password" : "text"}
            value={r.v}
            onChange={(e) => update(i, "v", e.target.value)}
            placeholder={valuePlaceholder}
            autoComplete="off"
          />
          <button className="mori-btn small ghost" onClick={() => remove(i)} title="刪除">✕</button>
        </div>
      ))}
      <button className="mori-btn small" onClick={add}>+ 新增</button>
    </div>
  );
}

// ─── helpers:JSON ↔ form state ──────────────────────────────────────

type AnyObj = Record<string, any>;

function getStr(obj: AnyObj | undefined, key: string, fallback = ""): string {
  const v = obj?.[key];
  return typeof v === "string" ? v : v == null ? fallback : String(v);
}

function setStrOrUndef(obj: AnyObj, key: string, value: string) {
  if (value === "") delete obj[key];
  else obj[key] = value;
}

function ensureSubObj(obj: AnyObj, key: string): AnyObj {
  if (!obj[key] || typeof obj[key] !== "object") obj[key] = {};
  return obj[key];
}

// brand-3: theme picker — 列 ~/.mori/themes/*.json 給 user 選 / reload。
// 內建 dark / light + 任何 user 自訂 json file 都會列出。
function ThemeSection() {
  const [themes, setThemes] = useState<ThemeEntry[]>([]);
  const [active, setActive] = useState<string>("dark");
  const [dir, setDir] = useState<string>("");
  const [busy, setBusy] = useState(false);

  const refresh = async () => {
    try {
      const list = await listThemes();
      setThemes(list);
    } catch (e) {
      console.error("[theme] list failed", e);
    }
    try {
      const res = await loadActiveTheme();
      if (res) setActive(res[0]);
    } catch (e) {
      console.error("[theme] active failed", e);
    }
  };

  useEffect(() => {
    refresh();
    themesDir().then(setDir).catch(console.error);
  }, []);

  const handleChange = async (stem: string) => {
    setBusy(true);
    try {
      await setActiveTheme(stem);
      setActive(stem);
    } catch (e) {
      console.error("[theme] set active failed", e);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Section
      title="Theme"
      hint="主視窗配色。內建 Mori Dark / Mori Light;放任何 *.json 到 themes 資料夾即可加入自訂 theme(VSCode-like)。"
    >
      <FormRow label="theme" hint="active 樣式">
        <Select
          value={active}
          onChange={handleChange}
          disabled={busy}
          options={themes.map((t) => ({
            value: t.stem,
            label: `${t.name}${t.builtin ? "" : "  (custom)"}  · ${t.base}`,
          }))}
        />
      </FormRow>
      <FormRow label="themes folder" hint="放 *.json 進去會列在上面下拉">
        <div className="mori-theme-path-row">
          <code className="mori-theme-path">{dir || "(loading…)"}</code>
          <button
            className="mori-btn small"
            onClick={refresh}
            disabled={busy}
            title="重新掃描資料夾"
          >Reload</button>
        </div>
      </FormRow>
    </Section>
  );
}

// 5E-3: VoiceInput inject_memory_types chips。同 ProfileEditor 的
// MemoryTypeChipsEditor 邏輯,獨立放這避免兩個 tsx 互相 import。
const INJECTABLE_MEMORY_TYPES = [
  "voice_dict",
  "preference",
  "user_identity",
  "project",
  "reference",
  "skill_outcome",
];

function ConfigMemoryTypeChips({
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
          type="button"
          className={`mori-skill-chip ${value.includes(t) ? "on" : ""}`}
          onClick={() => toggle(t)}
        >
          {value.includes(t) ? "✓ " : ""}{t}
        </button>
      ))}
    </div>
  );
}

// 5P-6: Character pack picker — Floating section 內,讓 user 切換 / 列出 / 升級
// 4×4 placeholder。換 active 後 emit "character-changed" 讓 FloatingMori 即時 re-fetch。
function CharacterPicker() {
  const [chars, setChars] = useState<CharacterEntry[]>([]);
  const [active, setActive] = useState<string>("mori");
  const [characterDir, setCharacterDir] = useState<string>("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const refresh = async () => {
    try {
      const list = await invoke<CharacterEntry[]>("character_list");
      setChars(list);
      const [stem] = await invoke<[string, unknown]>("character_get_active");
      setActive(stem);
      setCharacterDir(await invoke<string>("character_dir"));
    } catch (e) {
      console.error("CharacterPicker refresh", e);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const onSelect = async (stem: string) => {
    setBusy(true);
    setMsg(null);
    try {
      await invoke("character_set_active", { stem });
      setActive(stem);
      await emit("character-changed");
      setMsg(`已切換到 ${stem}`);
      setTimeout(() => setMsg(null), 2000);
    } catch (e: any) {
      setMsg(`切換失敗:${e}`);
    } finally {
      setBusy(false);
    }
  };

  const onUpgrade = async () => {
    setBusy(true);
    setMsg(null);
    try {
      const [up, sk] = await invoke<[number, number]>("character_upgrade_pack_to_4x4", {
        stem: active,
      });
      await emit("character-changed");
      setMsg(`升級完成:${up} 張升 4×4,${sk} 已是 1024×1024 略過`);
      setTimeout(() => setMsg(null), 4000);
    } catch (e: any) {
      setMsg(`升級失敗:${e}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <FormRow
        label="character"
        hint={`Active character pack — 切到要用的角色。資料夾: ${characterDir}`}
      >
        <Select
          value={active}
          onChange={onSelect}
          options={chars.map((c) => ({
            value: c.stem,
            label: `${c.display_name}${c.author ? ` · ${c.author}` : ""}`,
          }))}
        />
      </FormRow>
      <FormRow
        label=""
        hint="把 active pack 內 single-frame sprite 升 4×4 placeholder(原檔備份到 sprites/.backup-<ts>/)"
      >
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <button className="mori-btn" onClick={onUpgrade} disabled={busy}>
            升級此 pack 為 4×4 placeholder
          </button>
          {msg && <span style={{ fontSize: 12, opacity: 0.8 }}>{msg}</span>}
        </div>
      </FormRow>
    </>
  );
}

function ConfigTab() {
  const [raw, setRaw] = useState<string>("");
  const [orig, setOrig] = useState<string>("");
  // 5R-followup-4: sub-tab IA。raw 也是其中一個 sub-tab(取代舊的
  // form / raw 二選一 toggle)。
  const [subTab, setSubTab] = useState<SubTabId>("quick");
  // X11 session 偵測 — 用來條件 render「X11 only」sub-tab(Wayland 看不到)
  const [isX11, setIsX11] = useState(false);
  // Hotkey sub-tab 上的 session 標籤 + Wayland 提示文案要看實際 session type:
  // "x11" | "wayland" | "linux-other" | "non-linux"。
  const [sessionType, setSessionType] = useState<string>("non-linux");
  useEffect(() => {
    invoke<boolean>("is_x11_session").then(setIsX11).catch(() => {});
    invoke<string>("linux_session_type").then(setSessionType).catch(() => {});
  }, []);
  const [status, setStatus] = useState<SaveStatus>({ kind: "idle" });
  const [error, setError] = useState<string | null>(null);

  // corrections.md 同一頁編
  const [corrText, setCorrText] = useState<string>("");
  const [corrOrig, setCorrOrig] = useState<string>("");
  const [corrStatus, setCorrStatus] = useState<SaveStatus>({ kind: "idle" });

  useEffect(() => {
    invoke<string>("config_read")
      .then((t) => { setRaw(t); setOrig(t); })
      .catch((e) => setError(`load config.json: ${e}`));
    invoke<string>("corrections_read")
      .then((t) => { setCorrText(t); setCorrOrig(t); })
      .catch(() => {
        setCorrText("# Mori STT 校正表\n\n# 看到左邊 → 改成右邊\n# 例:modem -> Markdown\n\n");
      });
  }, []);

  // Parse raw JSON for form view(失敗則保留 form 為 default,raw view 顯紅框)
  const cfg: AnyObj = useMemo(() => {
    try { return JSON.parse(raw || "{}"); } catch { return {}; }
  }, [raw]);

  // raw view 的 JSON syntax 即時驗證
  const rawError = useMemo<string | null>(() => {
    if (!raw.trim()) return null;
    try { JSON.parse(raw); return null; } catch (e: any) { return e.message; }
  }, [raw]);

  // 從目前 cfg + patch 序列化回 raw。
  // 用 functional setState 避免 batched state 看到 stale raw closure。
  const applyPatch = (patch: (cfg: AnyObj) => void) => {
    setRaw((currentRaw) => {
      const next = JSON.parse(currentRaw || "{}");
      patch(next);
      return JSON.stringify(next, null, 2);
    });
  };

  const apiKeysRows: Array<{ k: string; v: string }> = useMemo(() => {
    const keys = cfg.api_keys;
    if (!keys || typeof keys !== "object") return [];
    return Object.entries(keys).map(([k, v]) => ({ k, v: String(v ?? "") }));
  }, [cfg]);

  const setApiKeys = (rows: Array<{ k: string; v: string }>) => {
    applyPatch((c) => {
      const obj: AnyObj = {};
      rows.forEach((r) => {
        if (r.k.trim()) obj[r.k.trim()] = r.v;
      });
      if (Object.keys(obj).length === 0) {
        delete c.api_keys;
      } else {
        c.api_keys = obj;
      }
    });
  };

  const routingSkillsRows: Array<{ k: string; v: string }> = useMemo(() => {
    const skills = cfg.routing?.skills;
    if (!skills || typeof skills !== "object") return [];
    return Object.entries(skills).map(([k, v]) => ({ k, v: String(v ?? "") }));
  }, [cfg]);

  const setRoutingSkills = (rows: Array<{ k: string; v: string }>) => {
    applyPatch((c) => {
      const r = ensureSubObj(c, "routing");
      const obj: AnyObj = {};
      rows.forEach((row) => {
        if (row.k.trim() && row.v.trim()) obj[row.k.trim()] = row.v.trim();
      });
      if (Object.keys(obj).length === 0) {
        delete r.skills;
        if (Object.keys(r).length === 0) delete c.routing;
      } else {
        r.skills = obj;
      }
    });
  };

  const saveConfig = async () => {
    if (rawError) { setStatus({ kind: "err", message: `JSON: ${rawError}` }); return; }
    setStatus({ kind: "saving" });
    try {
      await invoke("config_write", { text: raw });
      setOrig(raw);
      setStatus({ kind: "ok" });
      setTimeout(() => setStatus({ kind: "idle" }), 2500);
    } catch (e: any) {
      setStatus({ kind: "err", message: String(e) });
    }
  };

  const saveCorrections = async () => {
    setCorrStatus({ kind: "saving" });
    try {
      await invoke("corrections_write", { text: corrText });
      setCorrOrig(corrText);
      setCorrStatus({ kind: "ok" });
      setTimeout(() => setCorrStatus({ kind: "idle" }), 2500);
    } catch (e: any) {
      setCorrStatus({ kind: "err", message: String(e) });
    }
  };

  const dirty = raw !== orig;
  const corrDirty = corrText !== corrOrig;

  // 5R-followup-4: Sub-tab IA — 把 sections 按使用者心智模型分組,不再
  // 全部塞在一條 form 裡 scroll 累。X11 sub-tab 條件 render。
  const subTabs: SubTabSpec[] = [
    { id: "quick", label: "Quick setup", Icon: IconHome },
    { id: "llm", label: "LLM / Provider", Icon: IconCloud },
    { id: "voice", label: "Voice input", Icon: IconVoiceMic },
    { id: "appearance", label: "Appearance", Icon: IconTree },
    { id: "hotkey", label: "Hotkey", Icon: IconKeyboard },
    ...(isX11 ? [{ id: "x11" as SubTabId, label: "X11 only", Icon: IconKeyboard }] : []),
    { id: "corrections" as SubTabId, label: "Corrections", Icon: IconClipboard },
    { id: "raw", label: "Raw JSON", Icon: IconPencil },
  ];

  return (
    <div className="mori-tab mori-tab-config">
      <h2 className="mori-tab-title">Config</h2>

      {error && <div className="mori-config-error">{error}</div>}

      {/* Sticky save bar — 一個位置存所有 config.json fields。corrections.md
          有獨立 save 在它自己 sub-tab。 */}
      <div className="mori-config-savebar">
        <span className="mori-config-savebar-hint">
          ~/.mori/config.json · 改完不用重啟,下次熱鍵讀新值
        </span>
        <StatusBadge status={status} />
        <button
          className="mori-btn"
          onClick={() => setRaw(orig)}
          disabled={!dirty}
        >還原</button>
        <button
          className="mori-btn primary"
          onClick={saveConfig}
          disabled={!dirty || !!rawError}
          title="存 config.json"
        >儲存</button>
      </div>

      <div className="mori-config-layout">
        <SubTabNav
          active={subTab}
          onChange={setSubTab}
          tabs={subTabs}
          dirtyJson={!!rawError}
        />
        <div className="mori-config-content">
          {/* ── Quick setup ─────────────────────────────── */}
          {subTab === "quick" && <>
          <Section
            title="預設 Provider"
            hint="所有 profile 沒指定 provider 時用這個。VoiceInput profile 可以再 override 自己的 stt_provider。"
          >
            <FormRow label="provider" hint="主對話 / agent 用的 LLM">
              <Select
                value={getStr(cfg, "provider", "groq")}
                onChange={(v) => applyPatch((c) => setStrOrUndef(c, "provider", v))}
                options={ALL_PROVIDERS.map((p) => ({ value: p, label: p }))}
              />
            </FormRow>
            <FormRow label="stt_provider" hint="Whisper STT(語音轉文字)">
              <Select
                value={getStr(cfg, "stt_provider", "groq")}
                onChange={(v) => applyPatch((c) => setStrOrUndef(c, "stt_provider", v))}
                options={STT_PROVIDERS.map((p) => ({ value: p, label: p }))}
              />
            </FormRow>
          </Section>

          <Section
            title="API Keys"
            hint="這裡填或 OS 環境變數設都可以 — Mori 先看 OS env var,沒有再讀這份 map(env 優先)。Key 名照 *_API_KEY 慣例(GEMINI_API_KEY / OPENAI_API_KEY 等),值是密碼欄位。"
          >
            <KvTable
              rows={apiKeysRows}
              setRows={setApiKeys}
              keyPlaceholder="GEMINI_API_KEY"
              valuePlaceholder="key 值"
              valueIsSecret
            />
          </Section>
          </>}

          {/* ── LLM / Provider ──────────────────────────── */}
          {subTab === "llm" && <>
          <Section
            title="Provider 設定"
            hint="只列你會用的就好。空著的 provider 啟動時用內建預設(api_base / model 等)。"
          >
            <ProviderCard
              name="groq"
              cfg={cfg.providers?.groq}
              fields={[
                { key: "api_key", label: "api_key", secret: true, hint: "gsk_... — 這裡填或設 $GROQ_API_KEY env(env 優先)" },
                { key: "model", label: "model", hint: "openai/gpt-oss-120b" },
                { key: "stt_model", label: "stt_model", hint: "whisper-large-v3-turbo" },
              ]}
              onPatch={(patch) =>
                applyPatch((c) => {
                  const p = ensureSubObj(c, "providers");
                  const g = ensureSubObj(p, "groq");
                  patch(g);
                })
              }
            />
            <ProviderCard
              name="gemini"
              cfg={cfg.providers?.gemini}
              fields={[
                { key: "model", label: "model", hint: "gemini-3.1-flash-lite-preview" },
                { key: "api_base", label: "api_base", hint: "https://generativelanguage.googleapis.com/v1beta/openai/" },
              ]}
              hint="API key 在上方 API Keys 區填 GEMINI_API_KEY,或設 $GEMINI_API_KEY 環境變數(env 優先)。model / api_base 留空就用預設值。"
              onPatch={(patch) =>
                applyPatch((c) => {
                  const p = ensureSubObj(c, "providers");
                  const g = ensureSubObj(p, "gemini");
                  patch(g);
                })
              }
            />
            <ProviderCard
              name="ollama"
              cfg={cfg.providers?.ollama}
              fields={[
                { key: "base_url", label: "base_url", hint: "http://localhost:11434" },
                { key: "model", label: "model", hint: "qwen3:8b" },
              ]}
              onPatch={(patch) =>
                applyPatch((c) => {
                  const p = ensureSubObj(c, "providers");
                  const o = ensureSubObj(p, "ollama");
                  patch(o);
                })
              }
            />
            <ProviderCard
              name="claude-bash"
              cfg={cfg.providers?.["claude-bash"]}
              fields={[
                { key: "binary", label: "binary", hint: "PATH 上的 claude binary 名稱" },
                { key: "model", label: "model", hint: "(留空用 CLI 預設)" },
                { key: "mori_cli_path", label: "mori_cli_path", hint: "(留空自動偵測)" },
              ]}
              onPatch={(patch) =>
                applyPatch((c) => {
                  const p = ensureSubObj(c, "providers");
                  const b = ensureSubObj(p, "claude-bash");
                  patch(b);
                })
              }
            />
            <ProviderCard
              name="claude-cli"
              cfg={cfg.providers?.["claude-cli"]}
              fields={[
                { key: "binary", label: "binary", hint: "claude" },
                { key: "model", label: "model" },
              ]}
              onPatch={(patch) =>
                applyPatch((c) => {
                  const p = ensureSubObj(c, "providers");
                  const cc = ensureSubObj(p, "claude-cli");
                  patch(cc);
                })
              }
            />
            <ProviderCard
              name="whisper-local"
              cfg={cfg.providers?.["whisper-local"]}
              fields={[
                { key: "model_path", label: "model_path", hint: "~/.mori/models/ggml-small.bin(去 Deps 頁一鍵下載)" },
                { key: "server_binary", label: "server_binary", hint: "~/.mori/bin/whisper-server[.exe](去 Deps 頁一鍵下載,或填絕對路徑指向 GPU 版本)" },
                { key: "language", label: "language", hint: "zh / en / auto(留空 = auto detect)" },
              ]}
              hint="v2:Mori 自己不編 whisper.cpp,改 spawn 官方 whisper-server 子程序。模型 + 引擎兩件事都可以從 Deps 頁一鍵裝完,不用手動改 config。"
              onPatch={(patch) =>
                applyPatch((c) => {
                  const p = ensureSubObj(c, "providers");
                  const w = ensureSubObj(p, "whisper-local");
                  patch(w);
                })
              }
            />
          </Section>

          <Section
            title="Routing(進階)"
            hint="個別 skill 走不同 provider。沒設 = 全部用上面 provider。"
          >
            <FormRow label="agent" hint="agent loop 用哪個 provider(預設 = provider)">
              <Select
                value={cfg.routing?.agent ?? ""}
                allowEmpty
                emptyLabel="(同 provider)"
                onChange={(v) =>
                  applyPatch((c) => {
                    const r = ensureSubObj(c, "routing");
                    if (v === "") {
                      delete r.agent;
                      if (Object.keys(r).length === 0) delete c.routing;
                    } else {
                      r.agent = v;
                    }
                  })
                }
                options={ALL_PROVIDERS.map((p) => ({ value: p, label: p }))}
              />
            </FormRow>
            <FormRow label="skills" hint="skill_name → provider(空 = 用 agent / provider)">
              <KvTable
                rows={routingSkillsRows}
                setRows={setRoutingSkills}
                keyPlaceholder="translate / polish / summarize ..."
                valuePlaceholder="groq / claude-cli ..."
              />
            </FormRow>
          </Section>
          </>}

          {/* ── Voice input ────────────────────────────── */}
          {subTab === "voice" && <>
          <Section
            title="VoiceInput"
            hint="VoiceInput 模式的全域預設;每個 voice profile 都可以 override 自己這幾項。"
          >
            <FormRow label="cleanup_level" hint="smart=LLM+程式 / minimal=只程式 / none=raw 直貼">
              <Select
                value={cfg.voice_input?.cleanup_level ?? "smart"}
                onChange={(v) =>
                  applyPatch((c) => {
                    const vi = ensureSubObj(c, "voice_input");
                    vi.cleanup_level = v;
                  })
                }
                options={[
                  { value: "smart", label: "smart" },
                  { value: "minimal", label: "minimal" },
                  { value: "none", label: "none" },
                ]}
              />
            </FormRow>
            <FormRow
              label="inject_memory_types"
              hint="cleanup LLM 注入哪些 memory type 當校正詞庫(profile 沒設時的全域 default)"
            >
              <ConfigMemoryTypeChips
                value={Array.isArray(cfg.voice_input?.inject_memory_types)
                  ? cfg.voice_input!.inject_memory_types
                  : []}
                onChange={(next) =>
                  applyPatch((c) => {
                    const v = ensureSubObj(c, "voice_input");
                    if (next.length === 0) {
                      delete v.inject_memory_types;
                      if (Object.keys(v).length === 0) delete c.voice_input;
                    } else {
                      v.inject_memory_types = next;
                    }
                  })
                }
              />
            </FormRow>
          </Section>
          </>}

          {/* ── Appearance ─────────────────────────────── */}
          {subTab === "appearance" && <>
          <ThemeSection />
          <Section
            title="Floating Mori"
            hint="桌面 floating widget 的視覺行為。Sprite 資產走 character pack(~/.mori/characters/<active>/)。"
          >
            <FormRow
              label="animated"
              hint="動態 Mori — sprite 跑 4×4 sheet animation;關掉只顯示 frame 1 靜止"
            >
              <input
                type="checkbox"
                checked={cfg.floating?.animated ?? true}
                onChange={(e) =>
                  applyPatch((c) => {
                    const f = ensureSubObj(c, "floating");
                    f.animated = e.target.checked;
                  })
                }
              />
            </FormRow>
            <FormRow
              label="wander"
              hint="讓 Mori 在桌面隨機走動(實驗性,需 animated ON;多螢幕只在 Mori 目前所在那台走)"
            >
              <input
                type="checkbox"
                checked={cfg.floating?.wander ?? false}
                onChange={(e) =>
                  applyPatch((c) => {
                    const f = ensureSubObj(c, "floating");
                    if (e.target.checked) f.wander = true;
                    else delete f.wander;
                  })
                }
              />
            </FormRow>
            <CharacterPicker />
          </Section>
          </>}

          {/* ── Hotkey ─────────────────────────────────── */}
          {subTab === "hotkey" && <>
          <Section
            title="Toggle 模式"
            hint="Ctrl+Alt+Space(預設 chord)按下後怎麼觸發錄音。改完按 儲存 即時生效,不必重啟。"
          >
            <FormRow
              label="toggle_mode"
              hint="toggle:按一下開錄、再按一下停錄。hold:按住開錄、放開停錄(像 push-to-talk)。"
            >
              <Select
                value={getStr(cfg.hotkeys, "toggle_mode", "toggle")}
                onChange={(v) =>
                  applyPatch((c) => {
                    const h = ensureSubObj(c, "hotkeys");
                    if (v === "toggle") delete h.toggle_mode;
                    else h.toggle_mode = v;
                    if (Object.keys(h).length === 0) delete c.hotkeys;
                  })
                }
                options={[
                  { value: "toggle", label: "toggle(一按切換)" },
                  { value: "hold", label: "hold(按住錄、放開停)" },
                ]}
              />
            </FormRow>
            <p className="mori-config-section-hint" style={{ marginTop: "0.4em" }}>
              兩種模式共用同一個 chord(預設 <code>Ctrl+Alt+Space</code>),只是按下 / 放開的解讀不同。
              按 <strong>儲存</strong> 後 Mori 立即重讀,下一次按鍵就走新模式。
            </p>
          </Section>

          <Section
            title="鍵位"
            hint={
              sessionType === "wayland"
                ? "Wayland 上實際鍵位由系統設定決定 — 改 config.json 沒用,要去 GNOME Settings → Keyboard 改。"
                : sessionType === "x11"
                ? "X11 上 config.json 是 source of truth,改完重啟生效。"
                : "Linux 以外 Mori 走 tauri-plugin-global-shortcut,鍵位仍寫死 Ctrl+Alt+Space。"
            }
          >
            <FormRow
              label="toggle"
              hint="主錄音 chord。預設 Ctrl+Alt+Space。"
            >
              <input
                type="text"
                className="mori-input"
                value={getStr(cfg.hotkeys, "toggle", "Ctrl+Alt+Space")}
                onChange={(e) =>
                  applyPatch((c) => {
                    const h = ensureSubObj(c, "hotkeys");
                    setStrOrUndef(h, "toggle", e.target.value);
                    if (Object.keys(h).length === 0) delete c.hotkeys;
                  })
                }
                placeholder="Ctrl+Alt+Space"
              />
            </FormRow>
            <FormRow
              label="cancel"
              hint="錄音中按這個丟掉音檔(不送 STT)。Transcribing / Responding 時 abort pipeline。"
            >
              <input
                type="text"
                className="mori-input"
                value={getStr(cfg.hotkeys, "cancel", "Ctrl+Alt+Escape")}
                onChange={(e) =>
                  applyPatch((c) => {
                    const h = ensureSubObj(c, "hotkeys");
                    setStrOrUndef(h, "cancel", e.target.value);
                    if (Object.keys(h).length === 0) delete c.hotkeys;
                  })
                }
                placeholder="Ctrl+Alt+Escape"
              />
            </FormRow>
            <FormRow
              label="picker"
              hint="開 profile picker overlay(方向鍵選 voice / agent profile)。"
            >
              <input
                type="text"
                className="mori-input"
                value={getStr(cfg.hotkeys, "picker", "Ctrl+Alt+P")}
                onChange={(e) =>
                  applyPatch((c) => {
                    const h = ensureSubObj(c, "hotkeys");
                    setStrOrUndef(h, "picker", e.target.value);
                    if (Object.keys(h).length === 0) delete c.hotkeys;
                  })
                }
                placeholder="Ctrl+Alt+P"
              />
            </FormRow>
            {sessionType === "wayland" && (
              <p
                className="mori-config-section-hint"
                style={{ marginTop: "0.4em", borderLeft: "3px solid var(--mori-accent, #888)", paddingLeft: "0.6em" }}
              >
                Wayland portal 規範:第一次 Mori 啟動時 compositor(GNOME / KDE / …)會把預設 chord 記成「使用者要的綁定」,
                之後改 config.json <strong>不會自動覆寫</strong>。要改實際鍵位:
                <br />• GNOME 走 Settings → Keyboard → View and Customize Shortcuts
                <br />• 或刪掉 <code>~/.local/share/xdg-desktop-portal/permissions</code> 讓 Mori 下次啟動重新走權限 dialog
              </p>
            )}
          </Section>
          </>}

          {/* ── X11 only ────────────────────────────────── */}
          {subTab === "x11" && isX11 && <>
          <Section
            title="Floating 外觀(X11)"
            hint="只在 X11 session 有效。Wayland 上 body 真透明、XShape 沒對應 API,這幾項都沒影響。"
          >
            <FormRow
              label="x11 shape"
              hint="floating window OS-level 形狀。改完 save 即時套用(XShape clip 重新計算 + 套上)。"
            >
              <Select
                value={cfg.floating?.x11_shape ?? "circle"}
                onChange={(v) =>
                  applyPatch((c) => {
                    const f = ensureSubObj(c, "floating");
                    f.x11_shape = v;
                  })
                }
                options={[
                  { value: "square", label: "正方(無圓角)" },
                  { value: "rounded", label: "圓角矩形" },
                  { value: "circle", label: "圓形(玻璃球)" },
                ]}
              />
            </FormRow>
            <FormRow
              label="x11 shape radius"
              hint="圓角矩形的角弧(px),只在 x11_shape = rounded 時用。1 ~ 80。"
            >
              <input
                type="number"
                min={1}
                max={80}
                value={cfg.floating?.x11_shape_radius ?? 16}
                onChange={(e) =>
                  applyPatch((c) => {
                    const f = ensureSubObj(c, "floating");
                    f.x11_shape_radius = Number(e.target.value) || 16;
                  })
                }
                style={{ width: 70 }}
              />
            </FormRow>
            <FormRow
              label="x11 backplate"
              hint="floating window 內部底圖。logo 模式可放自己 PNG 在 ~/.mori/floating/backplate-{dark,light}.png 取代預設 Mori logo,即時生效。"
            >
              <Select
                value={cfg.floating?.x11_backplate ?? "plain"}
                onChange={(v) =>
                  applyPatch((c) => {
                    const f = ensureSubObj(c, "floating");
                    f.x11_backplate = v;
                  })
                }
                options={[
                  { value: "plain", label: "素色(跟著 theme 漸層)" },
                  { value: "logo", label: "背板(美術 PNG / 自訂)" },
                ]}
              />
            </FormRow>
          </Section>
          </>}

          {/* ── Corrections.md(獨立檔,獨立 save) ────────── */}
          {subTab === "corrections" && <>
          <Section
            title="corrections.md"
            hint="共用 STT 校正詞表。Voice / Agent profile 用 #file: ../corrections.md 引用,LLM 看 system prompt 時讀進去。"
          >
            <textarea
              className="mori-config-textarea"
              spellCheck={false}
              value={corrText}
              onChange={(e) => setCorrText(e.target.value)}
              rows={20}
            />
            <div className="mori-config-actions">
              <button
                className="mori-btn primary"
                onClick={saveCorrections}
                disabled={!corrDirty}
                title="只存 corrections.md,不存 config.json"
              >儲存 corrections.md</button>
              <button
                className="mori-btn"
                onClick={() => setCorrText(corrOrig)}
                disabled={!corrDirty}
              >還原</button>
              <StatusBadge status={corrStatus} />
            </div>
          </Section>
          </>}

          {/* ── Raw JSON view ──────────────────────────── */}
          {subTab === "raw" && <>
          <Section
            title="Raw JSON"
            hint="整份 ~/.mori/config.json,給 power user 直接編 routing.skills / shell_skills 等沒在表單裡的進階欄位。"
          >
            <textarea
              className={`mori-config-textarea ${rawError ? "has-error" : ""}`}
              spellCheck={false}
              value={raw}
              onChange={(e) => setRaw(e.target.value)}
              rows={28}
            />
            {rawError && (
              <div className="mori-config-error">JSON parse error: {rawError}</div>
            )}
          </Section>
          </>}
        </div>
      </div>
    </div>
  );
}

// ─── Provider card ──────────────────────────────────────────────────

type ProviderField = {
  key: string;
  label: string;
  hint?: string;
  secret?: boolean;
};

function ProviderCard({
  name,
  cfg,
  fields,
  hint,
  onPatch,
}: {
  name: string;
  cfg: AnyObj | undefined;
  fields: ProviderField[];
  hint?: string;
  onPatch: (patch: (provider: AnyObj) => void) => void;
}) {
  const [collapsed, setCollapsed] = useState(true);
  const present = !!cfg && Object.keys(cfg).length > 0;
  return (
    <div className={`mori-provider-card ${collapsed ? "collapsed" : ""}`}>
      <div
        className="mori-provider-card-head"
        onClick={() => setCollapsed(!collapsed)}
      >
        <span className="mori-provider-name">{name}</span>
        {present && <span className="mori-provider-set">已設</span>}
        {hint && <span className="mori-provider-hint">{hint}</span>}
        <span className="mori-provider-toggle">{collapsed ? "▸" : "▾"}</span>
      </div>
      {!collapsed && (
        <div className="mori-provider-card-body">
          {fields.map((f) => (
            <FormRow key={f.key} label={f.label} hint={f.hint}>
              <input
                className="mori-input"
                type={f.secret ? "password" : "text"}
                autoComplete="off"
                value={cfg?.[f.key] == null ? "" : String(cfg[f.key])}
                onChange={(e) =>
                  onPatch((p) => {
                    if (e.target.value === "") delete p[f.key];
                    else p[f.key] = e.target.value;
                  })
                }
              />
            </FormRow>
          ))}
        </div>
      )}
    </div>
  );
}

export default ConfigTab;
