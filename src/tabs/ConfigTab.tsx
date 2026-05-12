// 5L-2: ~/.mori/config.json typed form + raw JSON 雙模式編輯器。
//
// 設計:
// - 預設 Form view:常用欄位 typed inputs / dropdowns
// - Raw JSON view:textarea + 即時 parse 驗證,給 power user 加 routing.skills
//   等進階欄位
// - 兩個 view 共用一個 JSON state source-of-truth;切換時自動 sync,
//   未列在 form 的 key 也會保留(round-trip 不丟資料)
// - 儲存:寫整個 JSON 物件

import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { listThemes, setActiveTheme, themesDir, loadActiveTheme, type ThemeEntry } from "../theme";
import { Select } from "../Select";

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
        {hint && <span className="mori-form-row-hint">{hint}</span>}
      </div>
      <div className="mori-form-row-input">{children}</div>
    </div>
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
  const update = (i: number, field: "k" | "v", value: string) => {
    const next = [...rows];
    next[i] = { ...next[i], [field]: value };
    setRows(next);
  };
  const remove = (i: number) => setRows(rows.filter((_, j) => j !== i));
  const add = () => setRows([...rows, { k: "", v: "" }]);
  return (
    <div className="mori-kv-table">
      {rows.map((r, i) => (
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
  const [view, setView] = useState<"form" | "raw">("form");
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
  // 5P debug:用 functional setState 避免 batched state 看到 stale raw closure。
  const applyPatch = (patch: (cfg: AnyObj) => void) => {
    setRaw((currentRaw) => {
      const next = JSON.parse(currentRaw || "{}");
      patch(next);
      const newRaw = JSON.stringify(next, null, 2);
      console.log("[applyPatch] raw changed?", newRaw !== currentRaw,
        "rawLen", currentRaw.length, "→", newRaw.length);
      return newRaw;
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

  return (
    <div className="mori-tab mori-tab-config">
      <h2 className="mori-tab-title">Config</h2>
      <p className="mori-tab-hint">
        編輯 ~/.mori/config.json + corrections.md。改完不需要重啟,下一次熱鍵
        會即時讀新設定。Form view 蓋常用欄位,Raw JSON view 給 routing.skills 等進階。
      </p>

      {error && <div className="mori-config-error">{error}</div>}

      {/* brand-3: theme picker(獨立於 config.json 編輯之上,因為它寫的是
          ~/.mori/active_theme 而不是 config.json) */}
      <ThemeSection />

      {/* ── View toggle ───────────────────────────────── */}
      <div className="mori-view-toggle">
        <button
          className={`mori-view-tab ${view === "form" ? "active" : ""}`}
          onClick={() => setView("form")}
        >
          Form
        </button>
        <button
          className={`mori-view-tab ${view === "raw" ? "active" : ""}`}
          onClick={() => setView("raw")}
        >
          Raw JSON {rawError ? "⚠" : ""}
        </button>
        <div className="mori-view-toggle-actions">
          <StatusBadge status={status} />
          {/* 5P debug: 視覺化 dirty state,排查 Save 不 enable */}
          <span style={{ fontSize: 11, opacity: 0.6, fontFamily: "ui-monospace, monospace" }}>
            dirty={dirty ? "Y" : "N"} rawLen={raw.length} origLen={orig.length}
          </span>
          <button
            className="mori-btn"
            onClick={() => setRaw(orig)}
            disabled={!dirty}
          >還原</button>
          <button
            className="mori-btn primary"
            onClick={saveConfig}
            disabled={!dirty || !!rawError}
          >儲存</button>
        </div>
      </div>

      {view === "form" ? (
        <>
          {/* ── Defaults ───────────────────────────────── */}
          <Section
            title="預設"
            hint="所有 profile 沒指定 provider 時用這個。VoiceInput profile 可以再 override 自己的 stt_provider。"
          >
            <FormRow label="provider" hint="主對話 / agent LLM">
              <Select
                value={getStr(cfg, "provider", "groq")}
                onChange={(v) => applyPatch((c) => setStrOrUndef(c, "provider", v))}
                options={ALL_PROVIDERS.map((p) => ({ value: p, label: p }))}
              />
            </FormRow>
            <FormRow label="stt_provider" hint="Whisper STT">
              <Select
                value={getStr(cfg, "stt_provider", "groq")}
                onChange={(v) => applyPatch((c) => setStrOrUndef(c, "stt_provider", v))}
                options={STT_PROVIDERS.map((p) => ({ value: p, label: p }))}
              />
            </FormRow>
          </Section>

          {/* ── API keys ───────────────────────────────── */}
          <Section
            title="API Keys"
            hint="OS env var 找不到時的 fallback。Key 名建議 *_API_KEY,值會以密碼欄位呈現。"
          >
            <KvTable
              rows={apiKeysRows}
              setRows={setApiKeys}
              keyPlaceholder="GEMINI_API_KEY"
              valuePlaceholder="key 值"
              valueIsSecret
            />
          </Section>

          {/* ── Providers ──────────────────────────────── */}
          <Section
            title="Provider 設定"
            hint="只列你會用的就好。空著的 provider 啟動時用內建預設(api_base / model 等)。"
          >
            <ProviderCard
              name="groq"
              cfg={cfg.providers?.groq}
              fields={[
                { key: "api_key", label: "api_key", secret: true, hint: "gsk_..." },
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
                { key: "api_base", label: "api_base", hint: "(留空用預設 google 端點)" },
              ]}
              hint="key 從 api_keys.GEMINI_API_KEY 或 OS env 取"
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
                { key: "model_path", label: "model_path", hint: "/home/.../ggml-small.bin" },
                { key: "language", label: "language", hint: "zh / en / ..." },
              ]}
              onPatch={(patch) =>
                applyPatch((c) => {
                  const p = ensureSubObj(c, "providers");
                  const w = ensureSubObj(p, "whisper-local");
                  patch(w);
                })
              }
            />
          </Section>

          {/* ── Routing(進階)───────────────────────────── */}
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

          {/* ── Voice input ────────────────────────────── */}
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
            {/* 5N: 移除 dead UI — 全域 voice_input.auto_enter / .paste_shortcut
                兩條都不會被 backend 讀(只讀 profile.frontmatter 對應 key)。
                每個 voice profile 自己在 ProfileEditor 內勾,別在 Config 全域設。 */}

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

          {/* ── Floating Mori ─────────────────────────── */}
          <Section
            title="Floating Mori"
            hint="桌面 floating widget(160×160)的視覺行為。Sprite / 角色資產走 character pack — ~/.mori/characters/<active>/。"
          >
            <FormRow
              label="animated"
              hint="動態 Mori — sprite 跑 4×4 sheet animation;關掉只顯示 frame 1 靜止"
            >
              <input
                type="checkbox"
                checked={cfg.floating?.animated ?? true}
                onChange={(e) => {
                  const v = e.target.checked;
                  console.log("[Floating] animated toggle →", v);
                  applyPatch((c) => {
                    const f = ensureSubObj(c, "floating");
                    f.animated = v;
                  });
                }}
              />
            </FormRow>
            <FormRow
              label="wander"
              hint="讓 Mori 在桌面隨機走動(實驗性,需 animated ON;走動 sprite 沒上來前先 placeholder)"
            >
              <input
                type="checkbox"
                checked={cfg.floating?.wander ?? false}
                onChange={(e) => {
                  const v = e.target.checked;
                  console.log("[Floating] wander toggle →", v);
                  applyPatch((c) => {
                    const f = ensureSubObj(c, "floating");
                    if (v) f.wander = true;
                    else delete f.wander;
                  });
                }}
              />
            </FormRow>
            <CharacterPicker />
          </Section>
        </>
      ) : (
        <Section title="" >
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
      )}

      {/* ── Corrections.md ────────────────────────────── */}
      <Section
        title="corrections.md"
        hint="共用 STT 校正表(voice / agent profile 用 #file: 引用)"
      >
        <textarea
          className="mori-config-textarea"
          spellCheck={false}
          value={corrText}
          onChange={(e) => setCorrText(e.target.value)}
          rows={14}
        />
        <div className="mori-config-actions">
          <button
            className="mori-btn primary"
            onClick={saveCorrections}
            disabled={!corrDirty}
          >儲存</button>
          <button
            className="mori-btn"
            onClick={() => setCorrText(corrOrig)}
            disabled={!corrDirty}
          >還原</button>
          <StatusBadge status={corrStatus} />
        </div>
      </Section>
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
