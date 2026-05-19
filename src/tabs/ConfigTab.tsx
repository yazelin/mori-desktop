// 5L-2: ~/.mori/config.json typed form + raw JSON 雙模式編輯器。
//
// 設計:
// - 預設 Form view:常用欄位 typed inputs / dropdowns
// - Raw JSON view:textarea + 即時 parse 驗證,給 power user 加 routing.skills
//   等進階欄位
// - 兩個 view 共用一個 JSON state source-of-truth;切換時自動 sync,
//   未列在 form 的 key 也會保留(round-trip 不丟資料)
// - 儲存:寫整個 JSON 物件

import React, { useEffect, useMemo, useState, type SVGProps } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { useTranslation } from "react-i18next";
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
  IconAnnuli,
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

// 已知的 named providers schema(typed fields)— 來源是 mori-core/src/llm/mod.rs。
// 任何在 config.json `providers.<name>` 但**不在**這 registry 的, render 時 fallback
// 到通用 KvTable editor(讓 user 自訂 OpenAI-compat 端點之類也能直接編輯)。
//
// 順序就是 UI 上 card 的展示順序。
const PROVIDER_SCHEMAS: Record<string, { fields: ProviderField[]; topHintKey?: string }> = {
  groq: {
    fields: [
      { key: "api_key", label: "api_key", secret: true, hint: "gsk_... — 這裡填或設 $GROQ_API_KEY env(env 優先)" },
      { key: "model", label: "model", hint: "openai/gpt-oss-120b" },
      { key: "stt_model", label: "stt_model", hint: "whisper-large-v3-turbo" },
    ],
  },
  gemini: {
    fields: [
      { key: "model", label: "model", hint: "gemini-3.1-flash-lite-preview" },
      { key: "api_base", label: "api_base", hint: "https://generativelanguage.googleapis.com/v1beta/openai/" },
    ],
    topHintKey: "config_tab.rows.hint_llm_gemini_key",
  },
  ollama: {
    fields: [
      { key: "base_url", label: "base_url", hint: "http://localhost:11434" },
      { key: "model", label: "model", hint: "qwen3:8b" },
    ],
  },
  "claude-bash": {
    fields: [
      { key: "binary", label: "binary", hint: "通常填 `claude`(已裝過就在 PATH 上)。完整路徑也可。" },
      { key: "model", label: "model", hint: "(留空用 claude CLI 內建預設)" },
      { key: "mori_cli_path", label: "mori_cli_path", hint: "(留空 Mori 自動偵測)" },
    ],
    topHintKey: "config_tab.rows.hint_llm_bash_proxy",
  },
  "claude-cli": {
    fields: [
      { key: "binary", label: "binary", hint: "通常填 `claude`" },
      { key: "model", label: "model" },
    ],
    topHintKey: "config_tab.rows.hint_llm_chat_proxy",
  },
  "gemini-bash": {
    fields: [
      { key: "binary", label: "binary", hint: "通常填 `gemini`(短名 Mori 自動探 .cmd shim,從 v0.4.0 起)" },
      { key: "model", label: "model", hint: "(留空用 gemini CLI 內建預設)" },
      { key: "mori_cli_path", label: "mori_cli_path", hint: "(留空 Mori 自動偵測)" },
    ],
    topHintKey: "config_tab.rows.hint_llm_bash_proxy",
  },
  "gemini-cli": {
    fields: [
      { key: "binary", label: "binary", hint: "通常填 `gemini`(同 gemini-bash)" },
      { key: "model", label: "model" },
    ],
    topHintKey: "config_tab.rows.hint_llm_chat_proxy",
  },
  "codex-bash": {
    fields: [
      { key: "binary", label: "binary", hint: "通常填 `codex`(Windows 需 v0.130+ JS 版,native 變體不支援 Win)" },
      { key: "model", label: "model", hint: "(留空用 codex CLI 內建預設)" },
      { key: "mori_cli_path", label: "mori_cli_path", hint: "(留空 Mori 自動偵測)" },
    ],
    topHintKey: "config_tab.rows.hint_llm_bash_proxy",
  },
  "codex-cli": {
    fields: [
      { key: "binary", label: "binary", hint: "通常填 `codex`(同 codex-bash)" },
      { key: "model", label: "model" },
    ],
    topHintKey: "config_tab.rows.hint_llm_chat_proxy",
  },
  "whisper-local": {
    fields: [
      { key: "model_path", label: "model_path", hint: "~/.mori/models/ggml-small.bin(去 Deps 頁一鍵下載)" },
      { key: "server_binary", label: "server_binary", hint: "~/.mori/bin/whisper-server[.exe](去 Deps 頁一鍵下載,或填絕對路徑指向 GPU 版本)" },
      { key: "language", label: "language", hint: "zh / en / auto(留空 = auto detect)" },
    ],
    topHintKey: "config_tab.rows.hint_llm_whisper_server",
  },
};

// 已知 provider 的 name 列表(給上方 dropdown 用)。
const KNOWN_PROVIDERS = Object.keys(PROVIDER_SCHEMAS);

const STT_PROVIDERS = ["groq", "whisper-local"] as const;

// type ProviderField defined further down, but PROVIDER_SCHEMAS above
// references it — TypeScript hoists type declarations so this works.

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
  const { t } = useTranslation();
  if (status.kind === "idle") return null;
  if (status.kind === "saving") return <span className="mori-save-status saving">{t("common.loading")}</span>;
  if (status.kind === "ok") return <span className="mori-save-status ok">{t("common.ok_saved")}</span>;
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
 *  改成 on-demand 提示,大幅減少 Config tab 垂直密度。
 *
 *  v0.4.2:popover 用 React Portal 渲染到 document.body — 之前 CSS-only
 *  absolute popover 會被 ancestor 的 overflow:hidden / scroll container 切掉
 *  (Config sticky subnav layout + tab scrollable 共同造成)。Portal 直接
 *  脫離 DOM 子樹,position:fixed + getBoundingClientRect 動態算位置就解。 */
function HintTooltip({ children }: { children: React.ReactNode }) {
  const { t } = useTranslation();
  const anchorRef = React.useRef<HTMLSpanElement>(null);
  const [open, setOpen] = React.useState(false);
  const [pos, setPos] = React.useState<{ top: number; left: number }>({ top: 0, left: 0 });

  const place = () => {
    const el = anchorRef.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    // popover 預設展開在 icon 下方 4px;若 viewport 右側不夠 280px,改靠右對齊
    const viewportW = window.innerWidth;
    const popoverW = 280;
    const left = Math.min(r.left, viewportW - popoverW - 8);
    setPos({ top: r.bottom + 4, left: Math.max(8, left) });
  };

  const show = () => {
    place();
    setOpen(true);
  };
  const hide = () => setOpen(false);

  return (
    <>
      <span
        ref={anchorRef}
        className="mori-hint"
        tabIndex={0}
        aria-label={t("config_tab.rows.help_aria")}
        onMouseEnter={show}
        onMouseLeave={hide}
        onFocus={show}
        onBlur={hide}
      >
        <span className="mori-hint-icon">ⓘ</span>
      </span>
      {open &&
        createPortal(
          <div
            className="mori-hint-popover portal"
            style={{ top: pos.top, left: pos.left }}
            // 滑進 popover 不會消失(讓 user 能複製文字)
            onMouseEnter={() => setOpen(true)}
            onMouseLeave={hide}
          >
            {children}
          </div>,
          document.body,
        )}
    </>
  );
}

type SubTabId =
  | "quick"
  | "llm"
  | "voice"
  | "appearance"
  | "hotkey"
  | "x11"
  | "annuli"
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
  const { t } = useTranslation();
  return (
    <nav className="mori-config-subnav">
      {tabs.map((tab) => {
        const Icon = tab.Icon;
        return (
          <button
            key={tab.id}
            type="button"
            className={`mori-config-subtab ${active === tab.id ? "active" : ""}`}
            onClick={() => onChange(tab.id)}
          >
            <span className="mori-config-subtab-icon">
              <Icon width={14} height={14} />
            </span>
            <span className="mori-config-subtab-label">{tab.label}</span>
            {dirtyJson && tab.id === "raw" && (
              <span className="mori-config-subtab-dirty" title={t("config_tab.rows.json_dirty_title")} />
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
  const { t } = useTranslation();
  // 之前用 props.rows 直接驅動 render — 但 parent 的 setRows callback 會把
  // 空 key 的 row 過濾掉(不能寫進 JSON object),導致按「+ 新增」加的空白
  // row 在下一個 render 就被父層 filter 掉、看起來像按鈕沒反應。
  //
  // 改用 internal state:KvTable 保有「正在編輯的」rows(含空白草稿),
  // 父層只收到非空 key 的 rows 寫進 cfg。第一次掛載從 props 初始化;之後
  // 只在 localRows 仍空時 sync from prop(處理 config 異步載入慢於 mount
  // 的 race) — user 開始編輯後不沖掉草稿。
  const [localRows, setLocalRows] = useState<Array<{ k: string; v: string }>>(rows);
  // Fix v0.3.2 follow-up: ConfigTab 第一次點進「快速設定」時 raw 還在 config_read
  // async 載入, apiKeysRows 是空陣列 → KvTable mount 把 [] 鎖進 localRows;
  // 之後 config 載完 rows 變非空 KvTable 不會 sync,user 看到空白以為沒設定。
  // 切走 sub-tab 再切回來 KvTable remount 才拿到。修:rows 從空變非空時 sync。
  useEffect(() => {
    if (localRows.length === 0 && rows.length > 0) {
      setLocalRows(rows);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [rows]);
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
          <button className="mori-btn small ghost" onClick={() => remove(i)} title={t("config_tab.rows.remove_button_title")}>✕</button>
        </div>
      ))}
      <button className="mori-btn small" onClick={add}>{t("config_tab.rows.add_button")}</button>
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
  const { t } = useTranslation();
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
      title={t("config_tab.sections.appearance_theme")}
      hint={t("config_tab.sections.appearance_theme_hint")}
    >
      <FormRow label="theme" hint={t("config_tab.rows.theme_active")}>
        <Select
          value={active}
          onChange={handleChange}
          disabled={busy}
          options={themes.map((theme) => ({
            value: theme.stem,
            label: `${theme.name}${theme.builtin ? "" : "  (custom)"}  · ${theme.base}`,
          }))}
        />
      </FormRow>
      <FormRow label="themes folder" hint={t("config_tab.rows.themes_folder")}>
        <div className="mori-theme-path-row">
          <code className="mori-theme-path">{dir || "(loading…)"}</code>
          <button
            className="mori-btn small"
            onClick={refresh}
            disabled={busy}
            title={t("config_tab.rows.themes_folder_rescan")}
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
  const { t } = useTranslation();
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
      setMsg(t("config_tab.rows.char_switch_ok", { stem }));
      setTimeout(() => setMsg(null), 2000);
    } catch (e: any) {
      setMsg(t("config_tab.rows.char_switch_fail", { e: String(e) }));
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
      setMsg(t("config_tab.rows.char_upgrade_ok", { up, sk }));
      setTimeout(() => setMsg(null), 4000);
    } catch (e: any) {
      setMsg(t("config_tab.rows.char_upgrade_fail", { e: String(e) }));
    } finally {
      setBusy(false);
    }
  };

  return (
    <>
      <FormRow
        label="character"
        hint={`${t("config_tab.rows.char_pack_hint")}${characterDir}`}
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
        hint={t("config_tab.rows.char_upgrade_hint")}
      >
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <button className="mori-btn" onClick={onUpgrade} disabled={busy}>
            {t("config_tab.rows.upgrade_pack_button")}
          </button>
          {msg && <span style={{ fontSize: 12, opacity: 0.8 }}>{msg}</span>}
        </div>
      </FormRow>
    </>
  );
}

const ALL_SUBTAB_IDS: SubTabId[] = [
  "quick", "llm", "voice", "appearance", "hotkey", "x11", "annuli", "corrections", "raw",
];

// ── Wake-ack 音效設定(Phase 3A.1.2)──────────────────────────────────────
//
// Listening mode 下 wake event 觸發後播的應答音。內建 5 個 bundled .wav 解壓到
// `~/.mori/wakeword/sounds/wake-ack-alternates/`,user 可:
//   - 點 [使用] 切換當前播的檔
//   - 點 ▶ 試聽
//   - 上傳自己錄的 .wav
//   - toggle 全部關掉

type WakeAckAlternate = {
  filename: string;
  size_bytes: number;
  is_active: boolean;
  is_bundled: boolean;
};

type WakeAckStatus = {
  enabled: boolean;
  active_filename: string | null;
  custom_path: string | null;
  alternates: WakeAckAlternate[];
};

// WakeAckSection 需要 cfg + applyPatch 才能讓「啟用」toggle 走 form 的 dirty
// tracking(不然 toggle 不會 enable 儲存按鈕)。其他檔案操作(set_active /
// upload / delete)仍走 IPC,因為它們牽涉檔案系統。
type WakeAckSectionProps = {
  cfg: AnyObj;
  applyPatch: (mutator: (c: AnyObj) => void) => void;
};

function WakeAckSection({ cfg, applyPatch }: WakeAckSectionProps) {
  // i18n keys 之後補,目前 hardcode 中文(跟其他 hardcode 的 hint 一樣)
  const [status, setStatus] = useState<WakeAckStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);
  const fileInputRef = React.useRef<HTMLInputElement>(null);

  const refresh = async () => {
    try {
      const s = await invoke<WakeAckStatus>("wake_ack_status");
      setStatus(s);
    } catch (e) {
      console.error("wake_ack_status", e);
      setMsg(`讀失敗:${String(e)}`);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const flashMsg = (m: string) => {
    setMsg(m);
    setTimeout(() => setMsg(null), 2500);
  };

  const onUse = async (filename: string) => {
    setBusy(true);
    try {
      await invoke("wake_ack_set_active", { filename });
      await refresh();
      flashMsg(`已切換到 ${filename}`);
    } catch (e) {
      flashMsg(`切換失敗:${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  const onPreview = async (filename: string | null) => {
    try {
      await invoke("wake_ack_preview", { filename });
    } catch (e) {
      flashMsg(`試聽失敗:${String(e)}`);
    }
  };

  const onDelete = async (filename: string) => {
    if (!confirm(`刪除 ${filename}?(只能刪自己上傳的,內建檔不會被刪)`)) return;
    setBusy(true);
    try {
      await invoke("wake_ack_delete_alternate", { filename });
      await refresh();
      flashMsg(`已刪除 ${filename}`);
    } catch (e) {
      flashMsg(`刪除失敗:${String(e)}`);
    } finally {
      setBusy(false);
    }
  };

  // 「啟用」改走 applyPatch — 跟其他 form 欄位一致,動到會 enable 儲存按鈕。
  // (不再 call wake_ack_set_enabled IPC,Save 統一寫 config.json)
  const onToggleEnabled = (enabled: boolean) => {
    applyPatch((c) => {
      const lm = ensureSubObj(c, "listening_mode");
      lm.wake_ack_enabled = enabled;
    });
  };

  const onUpload = async (file: File) => {
    if (!file.name.toLowerCase().endsWith(".wav")) {
      flashMsg("只支援 .wav 檔");
      return;
    }
    setBusy(true);
    try {
      const buf = await file.arrayBuffer();
      const bytes = Array.from(new Uint8Array(buf));
      await invoke("wake_ack_upload", { filename: file.name, bytes });
      await refresh();
      flashMsg(`已上傳 ${file.name}`);
    } catch (e) {
      flashMsg(`上傳失敗:${String(e)}`);
    } finally {
      setBusy(false);
      if (fileInputRef.current) fileInputRef.current.value = "";
    }
  };

  if (!status) {
    return (
      <Section title="Wake-ack 應答音(Hey Mori 後播的回應)">
        <p style={{ opacity: 0.6, fontSize: 13 }}>載入中...</p>
      </Section>
    );
  }

  return (
    <Section
      title="Wake-ack 應答音"
      hint="Listening mode 下,Hey Mori 被偵測到後播這個音檔(讓你知道可以開始說話)。先放完再開麥克風,避免被 mic 收回去污染 STT。"
    >
      <FormRow label="啟用" hint="關掉就完全靜音(只看 floating Mori 動畫提示)。改動後按上方「儲存」才寫入 config.json。">
        <input
          type="checkbox"
          checked={Boolean((cfg.listening_mode?.wake_ack_enabled) ?? true)}
          onChange={(e) => onToggleEnabled(e.target.checked)}
        />
      </FormRow>

      <FormRow label="目前播的檔" hint={`實際播放路徑:~/.mori/wakeword/sounds/wake-ack.wav。「使用」按鈕會把選的檔覆蓋到那個固定路徑。${status.custom_path ? `\n config 有 wake_ack_path override:${status.custom_path}` : ""}`}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{ fontFamily: "monospace", fontSize: 13 }}>
            {status.active_filename ?? "(未指定 — 用 wake-ack.wav 預設)"}
          </span>
          <button
            className="mori-btn small ghost"
            onClick={() => onPreview(null)}
            title="試聽當前播的檔"
          >
            ▶ 試聽
          </button>
        </div>
      </FormRow>

      <FormRow label="備選音檔" hint={`從這幾個 cp 過去 wake-ack.wav。內建檔不能刪,只能 override(可重新解壓)。自己錄/下載的 .wav 從下方上傳。`}>
        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
          {status.alternates.length === 0 && (
            <span style={{ opacity: 0.6, fontSize: 13 }}>(無備選 — 重啟 mori-tauri 會解壓 5 個內建)</span>
          )}
          {status.alternates.map((alt) => (
            <div
              key={alt.filename}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                padding: "4px 8px",
                background: alt.is_active ? "var(--mori-accent-bg, rgba(120,180,140,0.15))" : "transparent",
                borderRadius: 4,
              }}
            >
              <span
                style={{
                  fontFamily: "monospace",
                  fontSize: 13,
                  flex: 1,
                  fontWeight: alt.is_active ? 600 : 400,
                }}
              >
                {alt.is_active && "✓ "}
                {alt.filename}
                {alt.is_bundled && <span style={{ opacity: 0.5, fontSize: 11, marginLeft: 6 }}>內建</span>}
              </span>
              <span style={{ opacity: 0.5, fontSize: 11 }}>{(alt.size_bytes / 1024).toFixed(0)} KB</span>
              <button
                className="mori-btn small ghost"
                onClick={() => onPreview(alt.filename)}
                title="試聽"
                disabled={busy}
              >
                ▶
              </button>
              <button
                className="mori-btn small"
                onClick={() => onUse(alt.filename)}
                disabled={busy || alt.is_active}
                title={alt.is_active ? "已經是當前" : "設為當前播的檔"}
              >
                {alt.is_active ? "使用中" : "使用"}
              </button>
              {!alt.is_bundled && (
                <button
                  className="mori-btn small ghost"
                  onClick={() => onDelete(alt.filename)}
                  disabled={busy}
                  title="刪除(只能刪自己上傳的)"
                  style={{ color: "var(--mori-danger, #c66)" }}
                >
                  ✕
                </button>
              )}
            </div>
          ))}
        </div>
      </FormRow>

      <FormRow label="上傳自錄" hint="自己錄一段 .wav 當應答音(建議 0.5-1.5 秒,-16 LUFS 左右音量)">
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <input
            ref={fileInputRef}
            type="file"
            accept=".wav,audio/wav,audio/x-wav"
            disabled={busy}
            onChange={(e) => {
              const f = e.target.files?.[0];
              if (f) onUpload(f);
            }}
          />
          {msg && <span style={{ fontSize: 12, opacity: 0.8 }}>{msg}</span>}
        </div>
      </FormRow>
    </Section>
  );
}

// ── Phase 3E:聲紋辨識(Speaker verification)──────────────────────────
//
// 啟用後 wake event 觸發 + STT 前先過 voice embedding 比對,別人聲音 silent
// reject。需先「錄音註冊我的聲音」(~30s) + DepsTab 裝 resemblyzer。

type SpeakerIdStatus = {
  enrolled: boolean;
  path: string;
  size_bytes: number;
  enabled: boolean;
  threshold: number;
};

type SpeakerIdSectionProps = {
  cfg: AnyObj;
  applyPatch: (mutator: (c: AnyObj) => void) => void;
};

const ENROLL_SAMPLE_TEXT = `嗨,我是 Mori 的使用者,我在錄音註冊我的聲音,讓 Mori 認得我。
Hey Mori,你今天好嗎?今天天氣不錯,陽光很好,我喜歡咖啡跟茶。
我來念幾種不同語氣的句子:這是平常講話,這是問句嗎?還有強調的時候!
最後 Hey Mori,我念完了。如果還沒到 30 秒,我就繼續隨便聊一下今天做了什麼,
工作如何,有沒有遇到什麼好玩的事,或者就重複念剛剛那段都可以,重點是別停。`;
const ENROLL_SECONDS = 30;

/** Recording modal with pulsing red dot + countdown + progress bar.
 *  純 JS 計時(SetInterval),不依賴後端 event。recording 實際在 Python 子進程
 *  跑,30 秒固定,modal 跟著計時一起跑,結束關閉。 */
function EnrollmentModal({
  open,
  onCancel,
}: {
  open: boolean;
  onCancel?: () => void;
}) {
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    if (!open) {
      setElapsed(0);
      return;
    }
    const start = Date.now();
    const t = setInterval(() => {
      setElapsed(Math.min(ENROLL_SECONDS, (Date.now() - start) / 1000));
    }, 100);
    return () => clearInterval(t);
  }, [open]);
  if (!open) return null;
  const pct = (elapsed / ENROLL_SECONDS) * 100;
  const remaining = Math.max(0, ENROLL_SECONDS - elapsed);
  return createPortal(
    <div
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.55)",
        zIndex: 10000,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <style>{`
        @keyframes mori-rec-pulse {
          0%, 100% { transform: scale(1); opacity: 0.95; box-shadow: 0 0 0 0 rgba(220,60,60,0.7); }
          50% { transform: scale(1.18); opacity: 1; box-shadow: 0 0 0 16px rgba(220,60,60,0); }
        }
      `}</style>
      <div
        style={{
          background: "var(--mori-bg, #fff)",
          padding: 28,
          borderRadius: 12,
          maxWidth: 600,
          width: "92%",
          boxShadow: "0 20px 60px rgba(0,0,0,0.4)",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 14, marginBottom: 16 }}>
          <div
            style={{
              width: 18,
              height: 18,
              background: "#dc3c3c",
              borderRadius: "50%",
              animation: "mori-rec-pulse 1.2s ease-in-out infinite",
            }}
          />
          <h3 style={{ margin: 0, fontSize: 18 }}>錄音中 — 請念出下方文字</h3>
        </div>

        <div
          style={{
            padding: 14,
            background: "rgba(0,0,0,0.04)",
            borderRadius: 6,
            fontSize: 14,
            lineHeight: 1.85,
            whiteSpace: "pre-wrap",
            marginBottom: 18,
            maxHeight: 200,
            overflowY: "auto",
          }}
        >
          {ENROLL_SAMPLE_TEXT}
        </div>

        <div style={{ marginBottom: 8, display: "flex", justifyContent: "space-between", fontSize: 13 }}>
          <span>已錄 <strong>{elapsed.toFixed(1)}s</strong> / {ENROLL_SECONDS}s</span>
          <span>剩餘 <strong>{remaining.toFixed(1)}s</strong></span>
        </div>
        <div
          style={{
            height: 8,
            background: "rgba(0,0,0,0.08)",
            borderRadius: 4,
            overflow: "hidden",
            marginBottom: 14,
          }}
        >
          <div
            style={{
              width: `${pct}%`,
              height: "100%",
              background: "linear-gradient(90deg, #6a8c5a 0%, #8db077 100%)",
              transition: "width 0.1s linear",
            }}
          />
        </div>

        <p style={{ fontSize: 12, opacity: 0.7, lineHeight: 1.6, margin: "0 0 14px 0" }}>
          💡 念完還沒滿就<strong>繼續隨意聊</strong>(今天 / 工作 / 任何)或重複範本。
          <strong>別停</strong>。中間靜音會被 VAD 砍掉,降低 embedding 品質。
        </p>

        {onCancel && elapsed < ENROLL_SECONDS && (
          <div style={{ textAlign: "right" }}>
            <button
              className="mori-btn small ghost"
              onClick={onCancel}
              style={{ color: "var(--mori-danger, #c66)" }}
            >
              取消(放棄這次)
            </button>
          </div>
        )}
      </div>
    </div>,
    document.body,
  );
}

function SpeakerIdSection({ cfg, applyPatch }: SpeakerIdSectionProps) {
  const [status, setStatus] = useState<SpeakerIdStatus | null>(null);
  const [enrolling, setEnrolling] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  const refresh = async () => {
    try {
      const s = await invoke<SpeakerIdStatus>("speaker_id_status");
      setStatus(s);
    } catch (e) {
      console.error("speaker_id_status", e);
    }
  };

  useEffect(() => {
    refresh();
  }, []);

  const flashMsg = (m: string) => {
    setMsg(m);
    setTimeout(() => setMsg(null), 4000);
  };

  const onEnroll = async () => {
    if (
      !confirm(
        `準備錄音 30 秒註冊聲紋。\n\n按下確定後會跳出讀稿視窗,跟著念就好。\n\n要點:\n• 自然語速,不要朗讀腔\n• 念完還沒 30 秒 → 繼續隨意聊或重複範本(別停)\n• 跟實際叫 Mori 時同麥克風距離\n• 多種語氣(平淡 / 問句 / 強調)`
      )
    )
      return;
    setEnrolling(true);
    try {
      await invoke("speaker_id_enroll", { seconds: 30 });
      await refresh();
      flashMsg("✓ 聲紋註冊完成");
    } catch (e) {
      flashMsg(`註冊失敗:${String(e)}`);
    } finally {
      setEnrolling(false);
    }
  };

  const onClear = async () => {
    if (!confirm("清除已註冊的聲紋?清除後 wake event 不會 gate(任何人都能叫 Mori)。")) return;
    try {
      await invoke("speaker_id_clear");
      await refresh();
      flashMsg("✓ 已清除");
    } catch (e) {
      flashMsg(`清除失敗:${String(e)}`);
    }
  };

  return (
    <>
    <EnrollmentModal open={enrolling} />
    <Section
      title="聲紋辨識(只認你,Phase 3E)"
      hint="啟用後 wake event 觸發、user 講完之後,先用 resemblyzer 比對聲紋。只有 enrolled user 的聲音通過,別人 silent reject。需先在 Deps 頁裝「聲紋辨識 runtime」(~100MB)+ 點下方錄音註冊一次。預設 OFF。"
    >
      <FormRow label="enabled" hint="OFF → 任何人都能叫 Mori(現有行為)。ON → 只認你。沒 enrolled 就 ON 也不會 gate(避免鎖死)。每次 wake 多 ~200-500ms 延遲跑 embedding 比對。">
        <input
          type="checkbox"
          checked={Boolean(cfg.speaker_id?.enabled)}
          onChange={(e) =>
            applyPatch((c) => {
              const s = ensureSubObj(c, "speaker_id");
              s.enabled = e.target.checked;
            })
          }
        />
      </FormRow>
      <FormRow label="threshold" hint="Cosine similarity 0~1,越高越嚴(只認跟 enrollment 高度相似的聲音)。預設 0.7。同一人不同日 / 感冒 / 距離 mic 不同會掉到 0.65-0.75,設太高容易擋掉自己。設太低(<0.55)別人也通過。建議從 0.7 開始,實測微調。">
        <input
          type="number"
          min={0.3}
          max={0.99}
          step={0.05}
          value={Number(cfg.speaker_id?.threshold ?? 0.7)}
          onChange={(e) =>
            applyPatch((c) => {
              const s = ensureSubObj(c, "speaker_id");
              const n = Number(e.target.value);
              s.threshold = Number.isFinite(n) ? Math.max(0.3, Math.min(0.99, n)) : 0.7;
            })
          }
        />
      </FormRow>
      {!status?.enrolled && (
        <div
          style={{
            margin: "8px 12px",
            padding: 10,
            background: "var(--mori-accent-bg, rgba(120,180,140,0.08))",
            borderLeft: "3px solid var(--mori-forest, #6a8c5a)",
            borderRadius: 4,
            fontSize: 12,
            lineHeight: 1.6,
          }}
        >
          <strong>📖 30 秒讀稿範本</strong>(按開始後唸這段,或自由發揮類似長度):
          <pre
            style={{
              margin: "6px 0 4px 0",
              padding: 8,
              background: "rgba(0,0,0,0.05)",
              borderRadius: 3,
              whiteSpace: "pre-wrap",
              fontFamily: "inherit",
              fontSize: 12,
            }}
          >
{`嗨,我是 Mori 的使用者,我在錄音註冊我的聲音。
Hey Mori,你好嗎?今天天氣不錯,我喜歡咖啡跟茶。
我來念幾種不同語氣的句子:這是平常講話,這是問句嗎?
還有強調的時候!Hey Mori,辨識完成。`}
          </pre>
          <strong>準確訣竅:</strong>自然語速 / 中間別長停頓 / 跟實際叫 Mori 同麥克風距離 / 含「平淡 + 問句 + 強調」多種語氣。
        </div>
      )}
      <FormRow label="" hint={status?.enrolled ? `已註冊:${status.path}(${status.size_bytes} bytes)` : "尚未註冊 — 按下方錄音 30 秒"}>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <button
            className="mori-btn"
            onClick={onEnroll}
            disabled={enrolling}
            title="錄 30 秒講話,抽聲紋當作識別基準"
          >
            {enrolling ? "錄音中..." : status?.enrolled ? "🎙 重新註冊" : "🎙 錄音註冊我的聲音"}
          </button>
          {status?.enrolled && (
            <button
              className="mori-btn small ghost"
              onClick={onClear}
              disabled={enrolling}
              style={{ color: "var(--mori-danger, #c66)" }}
            >
              ✕ 清除
            </button>
          )}
          {msg && <span style={{ fontSize: 12, opacity: 0.8 }}>{msg}</span>}
        </div>
      </FormRow>
      {!status?.enrolled && cfg.speaker_id?.enabled && (
        <p style={{ fontSize: 12, opacity: 0.7, padding: "4px 12px", color: "var(--mori-warn, #d80)" }}>
          ⚠ enabled 但還沒 enrolled — wake event 不會 gate(任何人都能用)。先按上方錄音註冊。
        </p>
      )}
    </Section>
    </>
  );
}

function ConfigTab({
  pendingSubTab,
  onSubTabApplied,
}: {
  pendingSubTab?: string | null;
  onSubTabApplied?: () => void;
} = {}) {
  const { t } = useTranslation();
  const [raw, setRaw] = useState<string>("");
  const [orig, setOrig] = useState<string>("");
  // 5R-followup-4: sub-tab IA。raw 也是其中一個 sub-tab(取代舊的
  // form / raw 二選一 toggle)。
  const [subTab, setSubTab] = useState<SubTabId>("quick");
  // 接 MainShell 傳來的 pendingSubTab(其他 tab 用 emit("mori-nav") 跳過來時用)
  useEffect(() => {
    if (pendingSubTab && (ALL_SUBTAB_IDS as string[]).includes(pendingSubTab)) {
      setSubTab(pendingSubTab as SubTabId);
      onSubTabApplied?.();
    }
  }, [pendingSubTab, onSubTabApplied]);
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
      // C — annuli 熱重載:任何 config 存檔後都 invoke 一下,後端會比對 annuli
      // 子樹有沒有變、要不要重建 client。失敗不擋存檔流程,只 console warn。
      try {
        const msg = await invoke<string>("annuli_reload");
        console.info("[annuli] reload OK:", msg);
      } catch (e) {
        console.warn("[annuli] reload failed (config 還是有存):", e);
      }
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
    { id: "quick", label: t("config_tab.subtabs.quick"), Icon: IconHome },
    { id: "llm", label: t("config_tab.subtabs.llm"), Icon: IconCloud },
    { id: "voice", label: t("config_tab.subtabs.voice"), Icon: IconVoiceMic },
    { id: "appearance", label: t("config_tab.subtabs.appearance"), Icon: IconTree },
    { id: "hotkey", label: t("config_tab.subtabs.hotkey"), Icon: IconKeyboard },
    ...(isX11 ? [{ id: "x11" as SubTabId, label: t("config_tab.subtabs.x11"), Icon: IconKeyboard }] : []),
    { id: "annuli" as SubTabId, label: t("config_tab.subtabs.annuli"), Icon: IconAnnuli },
    { id: "corrections" as SubTabId, label: t("config_tab.subtabs.corrections"), Icon: IconClipboard },
    { id: "raw", label: t("config_tab.subtabs.raw"), Icon: IconPencil },
  ];

  return (
    <div className="mori-tab mori-tab-config">
      <h2 className="mori-tab-title">{t("config_tab.title")}</h2>

      {error && <div className="mori-config-error">{error}</div>}

      {/* Sticky save bar — 一個位置存所有 config.json fields。corrections.md
          有獨立 save 在它自己 sub-tab。 */}
      <div className="mori-config-savebar">
        <span className="mori-config-savebar-hint">
          {t("config_tab.savebar_hint")}
        </span>
        <StatusBadge status={status} />
        <button
          className="mori-btn"
          onClick={() => setRaw(orig)}
          disabled={!dirty}
        >{t("common.revert")}</button>
        <button
          className="mori-btn primary"
          onClick={saveConfig}
          disabled={!dirty || !!rawError}
          title="config.json"
        >{t("common.save")}</button>
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
            title={t("config_tab.sections.quick_provider")}
            hint={t("config_tab.sections.quick_provider_hint")}
          >
            <FormRow label="provider" hint={t("config_tab.rows.hint_quick_provider")}>
              <Select
                value={getStr(cfg, "provider", "groq")}
                onChange={(v) => applyPatch((c) => setStrOrUndef(c, "provider", v))}
                options={[
                  ...KNOWN_PROVIDERS,
                  ...Object.keys(cfg.providers ?? {}).filter((n) => !PROVIDER_SCHEMAS[n]),
                ].map((p) => ({ value: p, label: p }))}
              />
            </FormRow>
            <FormRow label="stt_provider" hint={t("config_tab.rows.hint_quick_stt")}>
              <Select
                value={getStr(cfg, "stt_provider", "groq")}
                onChange={(v) => applyPatch((c) => setStrOrUndef(c, "stt_provider", v))}
                options={STT_PROVIDERS.map((p) => ({ value: p, label: p }))}
              />
            </FormRow>
          </Section>

          <Section
            title={t("config_tab.sections.quick_apikeys")}
            hint={t("config_tab.sections.quick_apikeys_hint")}
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
            title={t("config_tab.sections.llm_providers")}
            hint={t("config_tab.sections.llm_providers_hint")}
          >
            {/* Render order: 先 schema 已知的(維持固定順序), 再 config 裡有但 schema 沒收錄的(自訂)*/}
            {(() => {
              const customs = Object.keys(cfg.providers ?? {}).filter(
                (n) => !PROVIDER_SCHEMAS[n],
              );
              const all = [...KNOWN_PROVIDERS, ...customs];
              return all.map((name) => {
                const schema = PROVIDER_SCHEMAS[name];
                const onPatch = (patch: (provider: AnyObj) => void) =>
                  applyPatch((c) => {
                    const p = ensureSubObj(c, "providers");
                    const sub = ensureSubObj(p, name);
                    patch(sub);
                  });
                if (schema) {
                  return (
                    <ProviderCard
                      key={name}
                      name={name}
                      cfg={cfg.providers?.[name]}
                      fields={schema.fields}
                      hint={schema.topHintKey ? t(schema.topHintKey) : undefined}
                      onPatch={onPatch}
                    />
                  );
                }
                return (
                  <CustomProviderCard
                    key={name}
                    name={name}
                    cfg={cfg.providers?.[name]}
                    onPatch={onPatch}
                    onDelete={() =>
                      applyPatch((c) => {
                        const p = ensureSubObj(c, "providers");
                        delete p[name];
                      })
                    }
                  />
                );
              });
            })()}
            <AddProviderButton
              existingNames={[
                ...KNOWN_PROVIDERS,
                ...Object.keys(cfg.providers ?? {}),
              ]}
              onAdd={(name) =>
                applyPatch((c) => {
                  const p = ensureSubObj(c, "providers");
                  ensureSubObj(p, name);
                })
              }
            />
          </Section>

          <Section
            title={t("config_tab.sections.llm_routing")}
            hint={t("config_tab.sections.llm_routing_hint")}
          >
            <FormRow label="agent" hint={t("config_tab.rows.hint_llm_routing_agent")}>
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
                options={[
                  ...KNOWN_PROVIDERS,
                  ...Object.keys(cfg.providers ?? {}).filter((n) => !PROVIDER_SCHEMAS[n]),
                ].map((p) => ({ value: p, label: p }))}
              />
            </FormRow>
            <FormRow label="skills" hint={t("config_tab.rows.hint_llm_routing_skills")}>
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
            title={t("config_tab.sections.voice_input")}
            hint={t("config_tab.sections.voice_input_hint")}
          >
            <FormRow label="startup_mode" hint={t("config_tab.rows.hint_startup_mode")}>
              <Select
                value={cfg.startup_mode ?? "voice_input"}
                onChange={(v) =>
                  applyPatch((c) => {
                    c.startup_mode = v;
                  })
                }
                options={[
                  { value: "voice_input", label: "voice_input(dictation,啟動即可用)" },
                  { value: "agent", label: "agent(對話模式,跟 Mori 互動)" },
                  { value: "background", label: "background(假寐,麥克風關)" },
                ]}
              />
            </FormRow>
            <FormRow label="cleanup_level" hint={t("config_tab.rows.hint_voice_cleanup")}>
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
            <FormRow label="trim_silence_enabled" hint={t("config_tab.rows.hint_voice_trim_silence")}>
              <input
                type="checkbox"
                checked={cfg.voice_input?.trim_silence_enabled ?? true}
                onChange={(e) =>
                  applyPatch((c) => {
                    const vi = ensureSubObj(c, "voice_input");
                    vi.trim_silence_enabled = e.target.checked;
                  })
                }
              />
            </FormRow>
            <FormRow label="trim_silence_min_ms" hint={t("config_tab.rows.hint_voice_trim_silence_min_ms")}>
              <input
                type="number"
                min={1}
                max={5000}
                step={50}
                value={Number(cfg.voice_input?.trim_silence_min_ms ?? 300)}
                onChange={(e) =>
                  applyPatch((c) => {
                    const vi = ensureSubObj(c, "voice_input");
                    const n = Number(e.target.value);
                    vi.trim_silence_min_ms = Number.isFinite(n) ? Math.max(1, Math.min(5000, Math.round(n))) : 300;
                  })
                }
              />
            </FormRow>
            <FormRow label="trim_silence_threshold" hint={t("config_tab.rows.hint_voice_trim_silence_threshold")}>
              <input
                type="number"
                min={0.001}
                max={0.2}
                step={0.005}
                value={Number(cfg.voice_input?.trim_silence_threshold ?? 0.02)}
                onChange={(e) =>
                  applyPatch((c) => {
                    const vi = ensureSubObj(c, "voice_input");
                    const n = Number(e.target.value);
                    vi.trim_silence_threshold = Number.isFinite(n) ? Math.max(0.001, Math.min(0.2, n)) : 0.02;
                  })
                }
              />
            </FormRow>
            <FormRow label="min_audio_rms" hint={t("config_tab.rows.hint_voice_min_audio_rms")}>
              <input
                type="number"
                min={0.001}
                max={0.2}
                step={0.001}
                value={Number(cfg.voice_input?.min_audio_rms ?? 0.012)}
                onChange={(e) =>
                  applyPatch((c) => {
                    const vi = ensureSubObj(c, "voice_input");
                    const n = Number(e.target.value);
                    vi.min_audio_rms = Number.isFinite(n) ? Math.max(0.001, Math.min(0.2, n)) : 0.012;
                  })
                }
              />
            </FormRow>

            <FormRow
              label="inject_memory_types"
              hint={t("config_tab.rows.hint_voice_inject")}
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

          {/* ── Listening mode 偵測 + VAD silence-stop 設定 ── */}
          <Section
            title="Hey Mori 偵測 + 錄音"
            hint="Wake-word 偵測靈敏度 + Phase 3B 起的 VAD 自動停。預設 threshold 0.5 太敏感的話拉到 0.6~0.7。VAD 連續 silence_stop_secs 秒沒聲音自動停送 STT,max_record_secs 是安全兜底上限。"
          >
            <FormRow
              label="threshold"
              hint="偵測門檻(0.05~0.95)。越高越嚴格(必須完整「Hey Mori」才觸發),越低越敏感(誤觸多)。預設 0.5。建議從 0.65 試起。"
            >
              <input
                type="number"
                min={0.05}
                max={0.95}
                step={0.05}
                value={Number(cfg.listening_mode?.threshold ?? 0.5)}
                onChange={(e) =>
                  applyPatch((c) => {
                    const lm = ensureSubObj(c, "listening_mode");
                    const n = Number(e.target.value);
                    lm.threshold = Number.isFinite(n) ? Math.max(0.05, Math.min(0.95, n)) : 0.5;
                  })
                }
              />
            </FormRow>
            <FormRow
              label="silence_stop_secs"
              hint="VAD 連續多久靜音算 user 講完了(自動 stop)。預設 1.5s。太短會打斷思考停頓,太長會空等。clamp 0.3~10s。"
            >
              <input
                type="number"
                min={0.3}
                max={10}
                step={0.1}
                value={Number(cfg.listening_mode?.silence_stop_secs ?? 1.5)}
                onChange={(e) =>
                  applyPatch((c) => {
                    const lm = ensureSubObj(c, "listening_mode");
                    const n = Number(e.target.value);
                    lm.silence_stop_secs = Number.isFinite(n) ? Math.max(0.3, Math.min(10, n)) : 1.5;
                  })
                }
              />
            </FormRow>
            <FormRow
              label="silence_threshold_rms"
              hint="VAD 靜音判定 threshold(Recorder level 0~1)。低於此值算靜音。預設 0.012(對齊 voice_input.min_audio_rms)。背景吵就拉高到 0.02~0.05。"
            >
              <input
                type="number"
                min={0.001}
                max={0.2}
                step={0.005}
                value={Number(cfg.listening_mode?.silence_threshold_rms ?? 0.012)}
                onChange={(e) =>
                  applyPatch((c) => {
                    const lm = ensureSubObj(c, "listening_mode");
                    const n = Number(e.target.value);
                    lm.silence_threshold_rms = Number.isFinite(n) ? Math.max(0.001, Math.min(0.2, n)) : 0.012;
                  })
                }
              />
            </FormRow>
            <FormRow
              label="max_record_secs"
              hint="安全上限(秒)— 正常 VAD 偵測到靜音就先停了,這只是 VAD 沒 fire 時的兜底(背景持續噪音 / user 一直「ah...」沒停)。預設 30 秒。clamp 2~120。"
            >
              <input
                type="number"
                min={2}
                max={120}
                step={1}
                value={Number(cfg.listening_mode?.max_record_secs ?? 30)}
                onChange={(e) =>
                  applyPatch((c) => {
                    const lm = ensureSubObj(c, "listening_mode");
                    const n = Number(e.target.value);
                    lm.max_record_secs = Number.isFinite(n) ? Math.max(2, Math.min(120, Math.round(n))) : 30;
                  })
                }
              />
            </FormRow>
          </Section>

          {/* Wake-ack 應答音(獨立 Section,Phase 3A.1.2)*/}
          <WakeAckSection cfg={cfg} applyPatch={applyPatch} />

          {/* ── Phase 3E: 聲紋辨識(Speaker verification)── */}
          <SpeakerIdSection cfg={cfg} applyPatch={applyPatch} />

          {/* ── Phase 3C: Wake-event evaluator(背景噪音過濾)── */}
          <Section
            title="Wake-event 過濾(Evaluator,Phase 3C)"
            hint="Hey Mori 觸發後,STT 出來的 transcript 先過一輪 fast LLM 判斷:user 是不是真的在跟 Mori 講話?是 → 走正常 agent。否(自言自語 / 跟別人講話 / 念稿提到「Mori」)→ skip,不浪費 agent 跑無關內容。每次 wake 多 1 個 LLM call(~200ms,Groq 免費)。預設 OFF。"
          >
            <FormRow label="enabled" hint="OFF → 既有行為(直接進 agent)。ON → 多一層意圖判斷,過濾 wake-word false positive。建議 threshold 設低(0.35-0.5)但 evaluator ON,捕捉率 + 訊號比都高。">
              <input
                type="checkbox"
                checked={Boolean(cfg.evaluator?.enabled)}
                onChange={(e) =>
                  applyPatch((c) => {
                    const ev = ensureSubObj(c, "evaluator");
                    ev.enabled = e.target.checked;
                  })
                }
              />
            </FormRow>
            <FormRow label="provider" hint="跑 evaluator 用的 LLM provider。預設 groq(快、便宜、免費 quota 大)。模型走該 provider 的 model 設定(`providers.groq.model`)。Gemini API key 有的話也行,但 quota 更精貴不推薦。">
              <Select
                value={cfg.evaluator?.provider ?? "groq"}
                onChange={(v) =>
                  applyPatch((c) => {
                    const ev = ensureSubObj(c, "evaluator");
                    ev.provider = v;
                  })
                }
                options={[
                  { value: "groq", label: "groq(預設,gpt-oss-120b 快又便宜)" },
                  { value: "gemini", label: "gemini API(會吃 Gemini quota)" },
                  { value: "ollama", label: "ollama(本地,需自架)" },
                ]}
              />
            </FormRow>
          </Section>

          {/* ── Phase 3D: Mori 講話(edge-tts speak-back)── */}
          <Section
            title="Mori 講話(TTS,Phase 3D)"
            hint="Agent 回應完成後,讓 Mori 用聲音念出來。預設 OFF。用 Microsoft Edge 免費 TTS(無 quota、native zh-TW)。需先在 Deps 頁裝「Mori 講話 runtime(edge-tts)」。"
          >
            <FormRow label="enabled" hint="OFF → Mori 只在 ChatPanel 顯示文字(目前行為)。ON → 同時用聲音念。長回應 TTS 會講久,可能 5-15 秒,中途無法打斷(後續版本加 stop 鈕)。">
              <input
                type="checkbox"
                checked={Boolean(cfg.tts?.enabled)}
                onChange={(e) =>
                  applyPatch((c) => {
                    const t = ensureSubObj(c, "tts");
                    t.enabled = e.target.checked;
                  })
                }
              />
            </FormRow>
            <FormRow label="voice" hint="Microsoft Edge TTS 的 zh-TW voice。HsiaoChen 偏年輕清亮(預設,配 Mori 精靈少女形象),HsiaoYu 較成熟標準。換其他語言請查 `edge-tts --list-voices`。">
              <Select
                value={cfg.tts?.voice ?? "zh-TW-HsiaoChenNeural"}
                onChange={(v) =>
                  applyPatch((c) => {
                    const t = ensureSubObj(c, "tts");
                    t.voice = v;
                  })
                }
                options={[
                  // zh-TW(台灣腔)
                  { value: "zh-TW-HsiaoChenNeural", label: "🇹🇼 zh-TW-HsiaoChenNeural(女,年輕清亮,預設)" },
                  { value: "zh-TW-HsiaoYuNeural", label: "🇹🇼 zh-TW-HsiaoYuNeural(女,較成熟標準)" },
                  { value: "zh-TW-YunJheNeural", label: "🇹🇼 zh-TW-YunJheNeural(男)" },
                  // zh-CN(大陸腔)— Xiaoyi Lively 元氣感對 Mori 形象很合
                  { value: "zh-CN-XiaoyiNeural", label: "🇨🇳 zh-CN-XiaoyiNeural(女,活潑元氣)" },
                  { value: "zh-CN-XiaoxiaoNeural", label: "🇨🇳 zh-CN-XiaoxiaoNeural(女,溫暖)" },
                  { value: "zh-CN-liaoning-XiaobeiNeural", label: "🇨🇳 zh-CN-liaoning-XiaobeiNeural(女,東北腔幽默)" },
                  { value: "zh-CN-shaanxi-XiaoniNeural", label: "🇨🇳 zh-CN-shaanxi-XiaoniNeural(女,陝西腔明亮)" },
                  // zh-HK(粵語)
                  { value: "zh-HK-HiuGaaiNeural", label: "🇭🇰 zh-HK-HiuGaaiNeural(女,粵語)" },
                  { value: "zh-HK-HiuMaanNeural", label: "🇭🇰 zh-HK-HiuMaanNeural(女,粵語)" },
                  // 其他語言
                  { value: "ja-JP-NanamiNeural", label: "🇯🇵 ja-JP-NanamiNeural(日文女)" },
                  { value: "en-US-JennyNeural", label: "🇺🇸 en-US-JennyNeural(英文女)" },
                  { value: "en-US-AriaNeural", label: "🇺🇸 en-US-AriaNeural(英文女,活潑)" },
                ]}
              />
            </FormRow>
            <FormRow label="" hint="試聽當前 voice。會 spawn Python edge-tts 子進程,有點延遲(2-5 秒)是正常。">
              <button
                className="mori-btn"
                onClick={async () => {
                  try {
                    await invoke("tts_preview", {
                      text: "嗨,我是 Mori。試聽聲音這樣 OK 嗎?",
                      voice: cfg.tts?.voice ?? null,
                    });
                  } catch (e) {
                    alert(`試聽失敗:${String(e)}`);
                  }
                }}
              >
                ▶ 試聽
              </button>
            </FormRow>
          </Section>
          </>}

          {/* ── Appearance ─────────────────────────────── */}
          {subTab === "appearance" && <>
          <Section
            title={t("config_tab.sections.appearance_locale")}
            hint={t("config_tab.sections.appearance_locale_hint")}
          >
            <FormRow label="locale" hint={t("config_tab.rows.hint_locale")}>
              <Select
                value={getStr(cfg, "locale", "zh-TW")}
                onChange={(v) => {
                  applyPatch((c) => setStrOrUndef(c, "locale", v));
                  // 立即切 i18next(不用等 save 才看到)
                  import("../i18n").then((m) => m.setLocale(v as "zh-TW" | "en")).catch(() => {});
                }}
                options={[
                  { value: "zh-TW", label: "繁體中文 (zh-TW)" },
                  { value: "en", label: "English (en)" },
                ]}
              />
            </FormRow>
          </Section>
          <ThemeSection />
          <Section
            title={t("config_tab.sections.appearance_floating")}
            hint={t("config_tab.sections.appearance_floating_hint")}
          >
            <FormRow
              label={t("config_tab.rows.floating_show_mode")}
              hint={t("config_tab.rows.hint_floating_show_mode")}
            >
              <Select
                value={cfg.floating?.show_mode ?? "always"}
                options={[
                  { value: "always", label: t("config_tab.rows.floating_show_always") },
                  { value: "recording", label: t("config_tab.rows.floating_show_recording") },
                  { value: "off", label: t("config_tab.rows.floating_show_off") },
                ]}
                onChange={(v) =>
                  applyPatch((c) => {
                    const f = ensureSubObj(c, "floating");
                    f.show_mode = v;
                  })
                }
              />
            </FormRow>
            <FormRow
              label="animated"
              hint={t("config_tab.rows.hint_floating_animated")}
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
              hint={t("config_tab.rows.hint_floating_wander")}
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
            title={t("config_tab.sections.hotkey_toggle_mode")}
            hint={t("config_tab.sections.hotkey_toggle_mode_hint")}
          >
            <FormRow
              label="toggle_mode"
              hint={t("config_tab.rows.hint_hotkey_toggle_mode")}
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
            title={t("config_tab.sections.hotkey_keys")}
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
              hint={t("config_tab.rows.hint_hotkey_recording")}
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
              hint={t("config_tab.rows.hint_hotkey_cancel")}
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
              hint={t("config_tab.rows.hint_hotkey_picker")}
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
            title={t("config_tab.sections.x11_floating")}
            hint={t("config_tab.sections.x11_floating_hint")}
          >
            <FormRow
              label="x11 shape"
              hint={t("config_tab.rows.hint_x11_shape")}
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
              hint={t("config_tab.rows.hint_x11_radius")}
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
              hint={t("config_tab.rows.hint_x11_backplate")}
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

          {/* ── Annuli(vault-backed reflection engine) ────── */}
          {subTab === "annuli" && <>
          <Section
            title={t("config_tab.sections.annuli_connection")}
            hint={t("config_tab.sections.annuli_connection_hint")}
          >
            <FormRow label="enabled" hint={t("config_tab.rows.hint_annuli_enabled")}>
              <input
                type="checkbox"
                checked={cfg.annuli?.enabled ?? false}
                onChange={(e) =>
                  applyPatch((c) => {
                    const a = ensureSubObj(c, "annuli");
                    a.enabled = e.target.checked;
                  })
                }
              />
            </FormRow>
            <FormRow label="endpoint" hint={t("config_tab.rows.hint_annuli_endpoint")}>
              <input
                type="text"
                className="mori-input"
                placeholder="http://localhost:5000"
                value={cfg.annuli?.endpoint ?? ""}
                onChange={(e) =>
                  applyPatch((c) => {
                    const a = ensureSubObj(c, "annuli");
                    setStrOrUndef(a, "endpoint", e.target.value);
                  })
                }
              />
            </FormRow>
            <FormRow label="spirit_name" hint={t("config_tab.rows.hint_annuli_spirit")}>
              <input
                type="text"
                className="mori-input"
                placeholder="mori"
                value={cfg.annuli?.spirit_name ?? ""}
                onChange={(e) =>
                  applyPatch((c) => {
                    const a = ensureSubObj(c, "annuli");
                    setStrOrUndef(a, "spirit_name", e.target.value);
                  })
                }
              />
            </FormRow>
            <FormRow label="user_id" hint={t("config_tab.rows.hint_annuli_user_id")}>
              <input
                type="text"
                className="mori-input"
                placeholder="yazelin"
                value={cfg.annuli?.user_id ?? ""}
                onChange={(e) =>
                  applyPatch((c) => {
                    const a = ensureSubObj(c, "annuli");
                    setStrOrUndef(a, "user_id", e.target.value);
                  })
                }
              />
            </FormRow>
            <FormRow label="soul_token" hint={t("config_tab.rows.hint_annuli_soul_token")}>
              <input
                type="password"
                className="mori-input"
                placeholder={t("config_tab.rows.field_empty_readonly")}
                value={cfg.annuli?.soul_token ?? ""}
                onChange={(e) =>
                  applyPatch((c) => {
                    const a = ensureSubObj(c, "annuli");
                    setStrOrUndef(a, "soul_token", e.target.value);
                  })
                }
              />
            </FormRow>
            <FormRow label="timeout_secs" hint={t("config_tab.rows.hint_annuli_timeout")}>
              <input
                type="number"
                className="mori-input"
                min={1}
                max={600}
                placeholder="10"
                value={cfg.annuli?.timeout_secs ?? ""}
                onChange={(e) =>
                  applyPatch((c) => {
                    const a = ensureSubObj(c, "annuli");
                    const n = parseInt(e.target.value, 10);
                    if (isNaN(n)) {
                      delete a.timeout_secs;
                    } else {
                      a.timeout_secs = n;
                    }
                  })
                }
              />
            </FormRow>
          </Section>
          <Section
            title={t("config_tab.sections.annuli_basic_auth")}
            hint={t("config_tab.sections.annuli_basic_auth_hint")}
          >
            <FormRow label="user">
              <input
                type="text"
                className="mori-input"
                placeholder={t("config_tab.rows.field_empty_no_basic_auth")}
                value={cfg.annuli?.basic_auth?.user ?? ""}
                onChange={(e) =>
                  applyPatch((c) => {
                    const a = ensureSubObj(c, "annuli");
                    const v = e.target.value;
                    if (!v && !(a.basic_auth?.pass)) {
                      delete a.basic_auth;
                      return;
                    }
                    const ba = ensureSubObj(a, "basic_auth");
                    setStrOrUndef(ba, "user", v);
                  })
                }
              />
            </FormRow>
            <FormRow label="pass">
              <input
                type="password"
                className="mori-input"
                value={cfg.annuli?.basic_auth?.pass ?? ""}
                onChange={(e) =>
                  applyPatch((c) => {
                    const a = ensureSubObj(c, "annuli");
                    const v = e.target.value;
                    if (!v && !(a.basic_auth?.user)) {
                      delete a.basic_auth;
                      return;
                    }
                    const ba = ensureSubObj(a, "basic_auth");
                    setStrOrUndef(ba, "pass", v);
                  })
                }
              />
            </FormRow>
          </Section>
          </>}

          {/* ── Corrections.md(獨立檔,獨立 save) ────────── */}
          {subTab === "corrections" && <>
          <Section
            title={t("config_tab.sections.corrections_title")}
            hint={t("config_tab.sections.corrections_hint")}
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
                title={t("config_tab.rows.save_corrections_only_hint")}
              >{t("config_tab.rows.save_corrections")}</button>
              <button
                className="mori-btn"
                onClick={() => setCorrText(corrOrig)}
                disabled={!corrDirty}
              >{t("common.revert")}</button>
              <StatusBadge status={corrStatus} />
            </div>
          </Section>
          </>}

          {/* ── Raw JSON view ──────────────────────────── */}
          {subTab === "raw" && <>
          <Section
            title={t("config_tab.sections.raw_title")}
            hint={t("config_tab.sections.raw_hint")}
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

// 自訂 / 未列在 PROVIDER_SCHEMAS 的 provider — 用通用 KvTable 編輯任意 key/value,
// 適合 OpenAI-compat 自訂端點(`api_base` / `api_key_env` / `model` ...)。
function CustomProviderCard({
  name,
  cfg,
  onPatch,
  onDelete,
}: {
  name: string;
  cfg: AnyObj | undefined;
  onPatch: (patch: (provider: AnyObj) => void) => void;
  onDelete: () => void;
}) {
  const [collapsed, setCollapsed] = useState(true);
  const present = !!cfg && Object.keys(cfg).length > 0;
  const rows: Array<{ k: string; v: string }> = cfg
    ? Object.entries(cfg).map(([k, v]) => ({ k, v: v == null ? "" : String(v) }))
    : [];
  return (
    <div className={`mori-provider-card ${collapsed ? "collapsed" : ""}`}>
      <div
        className="mori-provider-card-head"
        onClick={() => setCollapsed(!collapsed)}
      >
        <span className="mori-provider-name">{name}</span>
        <span className="mori-provider-hint" style={{ opacity: 0.6 }}>自訂</span>
        {present && <span className="mori-provider-set">已設</span>}
        <span className="mori-provider-toggle">{collapsed ? "▸" : "▾"}</span>
      </div>
      {!collapsed && (
        <div className="mori-provider-card-body">
          <KvTable
            rows={rows}
            setRows={(newRows) => {
              onPatch((p) => {
                // 整個重寫:把舊 key 全清掉再放新的(否則刪除的 row 留在 JSON)。
                for (const k of Object.keys(p)) delete p[k];
                for (const { k, v } of newRows) {
                  if (k) p[k] = v;
                }
              });
            }}
            keyPlaceholder="api_base / api_key_env / model ..."
            valuePlaceholder=""
          />
          <button
            className="mori-btn small ghost"
            style={{ marginTop: 8 }}
            onClick={(e) => {
              e.stopPropagation();
              if (confirm(`刪除 provider "${name}"?\n(只清除 config.json 中此 provider 的條目)`)) {
                onDelete();
              }
            }}
          >
            刪除此 provider
          </button>
        </div>
      )}
    </div>
  );
}

// 「+ 新增 provider」按鈕 — 點開後輸入名稱, 按 Enter 加入(空 object), 接著可用
// CustomProviderCard 編輯其欄位。
function AddProviderButton({
  existingNames,
  onAdd,
}: {
  existingNames: string[];
  onAdd: (name: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState("");
  const trimmed = name.trim();
  const dup = trimmed && existingNames.includes(trimmed);
  if (!editing) {
    return (
      <button
        className="mori-btn small"
        style={{ marginTop: 8 }}
        onClick={() => {
          setEditing(true);
          setName("");
        }}
      >
        + 新增 provider
      </button>
    );
  }
  return (
    <div style={{ display: "flex", gap: 8, marginTop: 8, alignItems: "center" }}>
      <input
        className="mori-input"
        autoFocus
        placeholder="provider 名稱 (e.g. azure-gpt41)"
        value={name}
        onChange={(e) => setName(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && trimmed && !dup) {
            onAdd(trimmed);
            setEditing(false);
          } else if (e.key === "Escape") {
            setEditing(false);
          }
        }}
      />
      <button
        className="mori-btn small"
        disabled={!trimmed || !!dup}
        onClick={() => {
          onAdd(trimmed);
          setEditing(false);
        }}
      >
        加入
      </button>
      <button className="mori-btn small ghost" onClick={() => setEditing(false)}>
        取消
      </button>
      {dup && <span style={{ color: "var(--danger, #c00)", fontSize: 12 }}>已存在</span>}
    </div>
  );
}

export default ConfigTab;
