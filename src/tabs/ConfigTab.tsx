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
 *  改成 on-demand 提示,大幅減少 Config tab 垂直密度。 */
function HintTooltip({ children }: { children: React.ReactNode }) {
  const { t } = useTranslation();
  return (
    <span className="mori-hint" tabIndex={0} aria-label={t("config_tab.rows.help_aria")}>
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
                options={ALL_PROVIDERS.map((p) => ({ value: p, label: p }))}
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
              hint={t("config_tab.rows.hint_llm_gemini_key")}
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
              hint={t("config_tab.rows.hint_llm_whisper_server")}
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
                options={ALL_PROVIDERS.map((p) => ({ value: p, label: p }))}
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

export default ConfigTab;
