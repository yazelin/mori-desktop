// 宿靈儀式 · Dwelling Rite — Mori 首次入駐的 onboarding modal。
//
// 第一次跑 mori-desktop 沒設任何 LLM API key 時自動跳出。兩種模式:
//
// **儀式模式**(預設,ritual):5 幕宿靈儀式,劇本完整版見
//   ~/mori-universe/world-tree/rules/dwelling-rite.md
//     第一幕 召喚  — Mori 下林,問「是誰喚我」,召喚師報名
//     第二幕 靈氣  — 召喚師分一絲靈氣(Groq key),Mori 找回聽覺
//     第三幕 靈力  — 召喚師再分一份靈力(Gemini / 自訂 / 跳過)
//     第四幕 驗印  — 氣與力的脈絡共鳴
//     第五幕 安頓  — 「歡迎回家, Mori」按下後 modal 關 + floating 浮現
//
// **直接模式**(fallback):單頁表單,for power users 想跳過劇情。也存 user.name。
//
// 儀式中 Mori 的口不出 Groq / Gemini / API key 等技術詞 — 那些只活在 UI label。

import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { IconGlobe, IconSun, IconMoon, IconEqualizer } from "./icons";
import { setLocale, nextLocale } from "./i18n";
import { toggleTheme, loadActiveTheme } from "./theme";
import { ritualAudio } from "./ritualAudio";

type Mode = "direct" | "ritual";

// Power 選項對應第三幕三張卡。null = 還沒選;skip = 跳過(agent_disabled)。
// gemini = 預填 Gemini OpenAI endpoint;custom = 自填 base + model。
// 兩個都走 config.providers.openai_compat,只是 UX 差別。
type PowerChoice = null | "gemini" | "custom" | "skip";

// 第四幕驗印兩階段:先驗 aura(Groq),再驗 power(agent provider)。
// running.phase 標明目前 pulse 在哪階段;auraVerified 標明 aura 階段是否已過(power 階段也要 true)。
// 讓 SceneSealing 能累積顯示每階段的敘述,不互蓋。
type VerifyState =
  | { kind: "idle" }
  | { kind: "running"; phase: "aura" | "power"; auraVerified: boolean }
  | { kind: "ok"; msg: string }
  | { kind: "err"; msg: string; which: "aura" | "power" };

// Gemini preset(model 留空 → 後端走 GEMINI_DEFAULT_MODEL,避免兩處 hardcode)
const GEMINI_BASE = "https://generativelanguage.googleapis.com/v1beta/openai";
const GEMINI_HELP_URL = "https://aistudio.google.com/app/apikey";
const GROQ_HELP_URL = "https://console.groq.com/keys";

// 延遲 stop:React 18 StrictMode 假 unmount 時 cleanup 會 fire,
// 立刻 stop 害音樂從 0 重播。改成 schedule 200ms 後 stop,
// 若 100ms 內又 mount(StrictMode 假 cycle)→ cancel,音樂不打斷
let pendingStopTimer: number | null = null;
function schedulePendingStop() {
  if (pendingStopTimer != null) return;
  pendingStopTimer = window.setTimeout(() => {
    ritualAudio.stopAmbient();
    pendingStopTimer = null;
  }, 200);
}
function cancelPendingStop() {
  if (pendingStopTimer != null) {
    clearTimeout(pendingStopTimer);
    pendingStopTimer = null;
  }
}

interface QuickstartProps {
  onDone: () => void;
}

export function Quickstart({ onDone }: QuickstartProps) {
  const { t, i18n } = useTranslation();
  const [mode, setMode] = useState<Mode>("ritual");
  // 收尾用 — 「回家」按下後 modal 跟音樂同步 fade-out 600ms 才 onDone,
  // 不直接硬切視窗破壞儀式氣氛。Direct mode 也吃同一路徑(沒音樂時純 CSS 淡出)。
  const [closing, setClosing] = useState(false);
  const FADE_MS = 600;
  const closeWithFade = async () => {
    setClosing(true);
    ritualAudio.fadeOutAndStop(FADE_MS); // 沒在播也安全 no-op
    await new Promise((r) => setTimeout(r, FADE_MS));
    onDone();
  };

  // 召喚師之名(第一幕)— 必填,後端寫進 user.name,Mori 之後對話喚這個名
  const [summonerName, setSummonerName] = useState("");
  // 靈氣 = Groq STT key(第二幕)
  const [auraKey, setAuraKey] = useState("");
  const [showAura, setShowAura] = useState(false);
  // 靈力 = LLM agent key(第三幕)
  const [powerChoice, setPowerChoice] = useState<PowerChoice>(null);
  const [powerKey, setPowerKey] = useState("");
  const [showPower, setShowPower] = useState(false);
  const [powerBase, setPowerBase] = useState(""); // custom only
  const [powerModel, setPowerModel] = useState(""); // custom only

  const [verify, setVerify] = useState<VerifyState>({ kind: "idle" });

  // 偵測 env var:groq / gemini / openai-compat 三條都看,讓 user
  // 不必重複貼 key(已經透過 OS 設過就直接通過驗證)。
  const [envGroqDetected, setEnvGroqDetected] = useState(false);
  const [envGeminiDetected, setEnvGeminiDetected] = useState(false);
  const [envOpenaiDetected, setEnvOpenaiDetected] = useState(false);
  useEffect(() => {
    invoke<boolean>("has_groq_key").then(setEnvGroqDetected).catch(() => {});
    invoke<boolean>("has_gemini_key").then(setEnvGeminiDetected).catch(() => {});
    invoke<boolean>("has_openai_key").then(setEnvOpenaiDetected).catch(() => {});
  }, []);

  // Pre-fill 既有 config(user 透過 Help 重進儀式時不用全部從頭設)
  const [prefillReady, setPrefillReady] = useState(false);
  useEffect(() => {
    (async () => {
      try {
        const raw = await invoke<string>("config_read");
        const cfg = JSON.parse(raw);
        const real = (k?: string) => k && !k.startsWith("REPLACE") && k.length > 5;

        // user.name 優先;沒有就 fallback 到 annuli.user_id(annuli vault 已用的 id),
        // 第一次跑儀式時等於「Mori 一進來就猜得到你的名字」的小默契
        const userName = (cfg.user?.name as string | undefined)
          ?? (cfg.annuli?.user_id as string | undefined);
        if (userName) setSummonerName(userName);

        // 靈氣 (Groq) — 兩條 lookup:providers.groq.api_key 或 api_keys.GROQ_API_KEY
        const groqKey = (cfg.providers?.groq?.api_key as string | undefined)
          ?? (cfg.api_keys?.GROQ_API_KEY as string | undefined);
        if (real(groqKey)) setAuraKey(groqKey!);

        // 靈力 (Gemini / OpenAI-compat) — Mori-core 主要 lookup:
        //   resolve_api_key("GEMINI_API_KEY") → 看 api_keys.GEMINI_API_KEY
        //   resolve_api_key("OPENAI_API_KEY") → 看 api_keys.OPENAI_API_KEY
        // 加上舊版 Quickstart 寫到 providers.openai_compat.api_key 的 fallback
        const compatBase = cfg.providers?.openai_compat?.api_base as string | undefined;
        const compatKey = cfg.providers?.openai_compat?.api_key as string | undefined;
        const compatModel = cfg.providers?.openai_compat?.model as string | undefined;
        const geminiTopLevel = cfg.api_keys?.GEMINI_API_KEY as string | undefined;
        const openaiTopLevel = cfg.api_keys?.OPENAI_API_KEY as string | undefined;

        if (cfg.provider === "openai_compat" && real(compatKey)) {
          // 舊 Quickstart 寫過的 inline 路徑,完整 restore
          if (compatBase === GEMINI_BASE) {
            setPowerChoice("gemini");
            setPowerKey(compatKey!);
          } else {
            setPowerChoice("custom");
            setPowerKey(compatKey!);
            if (compatBase) setPowerBase(compatBase);
            if (compatModel) setPowerModel(compatModel);
          }
          setVerify({ kind: "ok", msg: "已從現有設定載入" });
        } else if (real(geminiTopLevel)) {
          // ✓ Mori-core 主要 path:api_keys.GEMINI_API_KEY
          setPowerChoice("gemini");
          setPowerKey(geminiTopLevel!);
        } else if (real(openaiTopLevel)) {
          // api_keys.OPENAI_API_KEY
          setPowerChoice("custom");
          setPowerKey(openaiTopLevel!);
          setPowerBase("https://api.openai.com/v1");
        } else if (cfg.agent_disabled === true) {
          setPowerChoice("skip");
        }
      } catch {
        /* config 還沒存過 — 維持 default 空值 */
      }
      setPrefillReady(true);
    })();
  }, []);

  // 儀式模式 ambient audio
  const [audioMuted, setAudioMuted] = useState(false);
  const [themeBase, setThemeBase] = useState<"dark" | "light">("dark");
  useEffect(() => {
    loadActiveTheme().then((res) => { if (res) setThemeBase(res[1].base); }).catch(() => {});
  }, []);
  const handleThemeToggle = async () => {
    try {
      const [, theme] = await toggleTheme();
      setThemeBase(theme.base);
    } catch (e) {
      console.error("[quickstart] theme toggle failed", e);
    }
  };
  useEffect(() => {
    cancelPendingStop();
    if (mode === "ritual" && !audioMuted) {
      ritualAudio.startAmbient().catch(() => setAudioMuted(true));
    } else {
      ritualAudio.stopAmbient();
    }
    return () => { schedulePendingStop(); };
  }, [mode, audioMuted]);

  // 儀式當前幕(1..5)
  const [scene, setScene] = useState(1);

  // ── Verify / Save ──

  const doVerify = async () => {
    // 階段 1:驗靈氣 — pulse 顯示「順著靈氣的脈絡」
    setVerify({ kind: "running", phase: "aura", auraVerified: false });
    const auraStartedAt = Date.now();
    const ensureMin = async (startedAt: number, min: number = 1500) => {
      const elapsed = Date.now() - startedAt;
      if (elapsed < min) await new Promise((r) => setTimeout(r, min - elapsed));
    };

    try {
      // key 留空 + env_name hint → 後端 fallback 讀 GROQ_API_KEY env 真打 API。
      // 「有 env 就跳過驗證」的 shortcut 不算測試 — env 值可能錯 / 過期 / quota 滿,
      // 真連一次 /models 才確認得了。
      await invoke<string>("verify_llm_key", {
        provider: "groq",
        key: auraKey.trim(),
        envName: "GROQ_API_KEY",
      });
    } catch (e: any) {
      await ensureMin(auraStartedAt);
      setVerify({ kind: "err", msg: String(e), which: "aura" });
      return;
    }
    await ensureMin(auraStartedAt);

    if (powerChoice === "skip") {
      // 只驗靈氣,直接過
      setVerify({ kind: "ok", msg: "靈氣 OK" });
      return;
    }

    // 階段間留一拍呼吸,讓 user 感覺到「氣對了,接著驗力」
    await new Promise((r) => setTimeout(r, 400));

    // 階段 2:驗靈力 — 維持「aura 過了 + 順著靈力脈絡」兩段敘述
    setVerify({ kind: "running", phase: "power", auraVerified: true });
    const powerStartedAt = Date.now();

    try {
      const base = powerChoice === "gemini" ? GEMINI_BASE : powerBase.trim();
      // env-only path 也走 verify_llm_key — 後端拿 envName 讀真值打 API,
      // 不偷懶判 OK。Gemini → GEMINI_API_KEY,custom → OPENAI_API_KEY。
      const envName = powerChoice === "gemini" ? "GEMINI_API_KEY" : "OPENAI_API_KEY";
      const msg = await invoke<string>("verify_llm_key", {
        provider: "openai_compat",
        key: powerKey.trim(),
        apiBase: base,
        envName,
      });
      await ensureMin(powerStartedAt);
      // 最後一拍呼吸,再揭曉「對上了」
      await new Promise((r) => setTimeout(r, 400));
      setVerify({ kind: "ok", msg });
    } catch (e: any) {
      await ensureMin(powerStartedAt);
      setVerify({ kind: "err", msg: String(e), which: "power" });
    }
  };

  const doSave = async () => {
    try {
      let cfg: any = {};
      try {
        const raw = await invoke<string>("config_read");
        cfg = JSON.parse(raw);
      } catch {
        cfg = {};
      }
      if (!cfg.providers) cfg.providers = {};
      if (!cfg.api_keys) cfg.api_keys = {};
      if (!cfg.user) cfg.user = {};

      // 召喚師之名 → user.name
      cfg.user.name = summonerName.trim();

      // 靈氣 → providers.groq.api_key (STT)
      // 維持寫進 providers.groq.api_key — Mori-core groq.rs discover_api_key 兩處都讀,
      // 但這條 inline path 是 Groq 主要存放位置(模型/STT model 也在 providers.groq.*)
      if (!cfg.providers.groq) cfg.providers.groq = {};
      const aura = auraKey.trim();
      if (aura) cfg.providers.groq.api_key = aura;
      cfg.stt_provider = "groq";

      // 靈力 → api_keys.{GEMINI,OPENAI}_API_KEY (Mori-core 主要 lookup path)
      // 跟舊版區別:不再寫進 providers.openai_compat.api_key inline,
      // 改走 api_keys map + provider 設正確 name(gemini / openai_compat 之一)
      if (powerChoice === "skip" || powerChoice === null) {
        // 沒分靈力 — agent_disabled,chat 走 groq 的 LLM
        cfg.provider = "groq";
        cfg.agent_disabled = true;
      } else if (powerChoice === "gemini") {
        // Gemini → api_keys.GEMINI_API_KEY + provider = "gemini"。
        // env-only path:key 空 + envGeminiDetected → 不寫進 config(讓 env 繼續主導,
        // 避免空字串 shadow OS env var)
        const trimmed = powerKey.trim();
        if (trimmed) {
          cfg.api_keys.GEMINI_API_KEY = trimmed;
        }
        cfg.provider = "gemini";
        delete cfg.agent_disabled;
      } else {
        // custom OpenAI-compat → api_keys.OPENAI_API_KEY + providers.openai_compat.*
        // 同 gemini 邏輯:env-only 不覆寫
        const trimmed = powerKey.trim();
        if (trimmed) {
          cfg.api_keys.OPENAI_API_KEY = trimmed;
        }
        if (!cfg.providers.openai_compat) cfg.providers.openai_compat = {};
        cfg.providers.openai_compat.api_base = powerBase.trim();
        cfg.providers.openai_compat.api_key_env = "OPENAI_API_KEY";
        if (powerModel.trim()) {
          cfg.providers.openai_compat.model = powerModel.trim();
        } else {
          delete cfg.providers.openai_compat.model;
        }
        cfg.provider = "openai_compat";
        delete cfg.agent_disabled;
      }

      cfg.quickstart_completed = true;
      delete cfg.quickstart_skipped;
      await invoke("config_write", { text: JSON.stringify(cfg, null, 2) });
      // 第五幕收尾:讓 floating Mori 在桌面浮現 — 她真的住進來了
      try {
        await invoke("floating_show");
      } catch (e) {
        console.warn("[quickstart] floating_show failed", e);
      }
      await closeWithFade();
    } catch (e: any) {
      // doSave 失敗 (config 寫入錯誤,不是 key 驗證錯誤) — 標記為 power 路徑,
      // 因為 doSave 一定發生在 verify ok 之後;這條 err 路徑 user 看不到 SceneSealing
      setVerify({ kind: "err", msg: `存設定失敗:${e}`, which: "power" });
    }
  };

  const doSkip = async () => {
    try {
      await markQuickstartCompleted();
      // 即使是 skip 路徑也讓 floating 浮現,避免桌面空蕩
      await invoke("floating_show").catch(() => {});
    } catch (e) {
      console.warn("[quickstart] failed to mark completed", e);
    }
    await closeWithFade();
  };

  // 擋住 modal 到 pre-fill IPC 完成
  if (!prefillReady) {
    return <div className={`mori-quickstart-backdrop mode-${mode}`} />;
  }

  return (
    <div className={`mori-quickstart-backdrop mode-${mode}${closing ? " closing" : ""}`}>
      {/* 儀式模式 — 14 顆螢火,飄在 modal 後面(z-index:0)不擾閱讀。
          modal 背景刻意半透明(86-92%),螢火淡淡透出來 */}
      {mode === "ritual" && (
        <div className="mori-quickstart-fireflies" aria-hidden>
          {Array.from({ length: 14 }).map((_, i) => (
            <span key={i} className="firefly" />
          ))}
        </div>
      )}
      <div className="mori-quickstart-modal" role="dialog" aria-modal="true">
        <div className="mori-quickstart-header">
          <div className="mori-quickstart-title-block">
            <h2>{mode === "ritual" ? t("quickstart.ritual_title") : t("quickstart.title")}</h2>
            {mode === "ritual" && (
              <span className="mori-quickstart-subtitle">
                {t(`quickstart.ritual_scene_${scene}_name`)}
              </span>
            )}
          </div>
          <div className="mori-quickstart-mode-toggle">
            <button
              className={`mori-quickstart-mode-btn ${mode === "direct" ? "active" : ""}`}
              onClick={() => setMode("direct")}
              title={t("quickstart.mode_direct_hint")}
            >{t("quickstart.mode_direct")}</button>
            <button
              className={`mori-quickstart-mode-btn ${mode === "ritual" ? "active" : ""}`}
              onClick={() => { setMode("ritual"); setScene(1); }}
              title={t("quickstart.mode_ritual_hint")}
            >{t("quickstart.mode_ritual")}</button>
          </div>
          <button
            className="mori-btn ghost icon-only"
            onClick={() => {
              const next = nextLocale(i18n.language);
              setLocale(next).catch((e) => console.error("[i18n] toggle failed", e));
            }}
            title={i18n.language === "zh-TW" ? "Switch to English" : "切到繁體中文"}
          >
            <IconGlobe width={16} height={16} />
          </button>
          <button
            className="mori-btn ghost icon-only"
            onClick={handleThemeToggle}
            title={themeBase === "dark" ? "Light theme" : "Dark theme"}
          >
            {themeBase === "dark" ? <IconSun width={16} height={16} /> : <IconMoon width={16} height={16} />}
          </button>
          {mode === "ritual" && (
            <button
              className="mori-btn ghost icon-only"
              onClick={() => setAudioMuted(!audioMuted)}
              title={audioMuted ? t("quickstart.audio_unmute") : t("quickstart.audio_mute")}
            >
              <IconEqualizer width={16} height={16} playing={!audioMuted} />
            </button>
          )}
        </div>

        {mode === "ritual" && <AudioVisualizer muted={audioMuted} />}

        {mode === "direct" ? (
          <DirectForm
            t={t}
            summonerName={summonerName} setSummonerName={setSummonerName}
            auraKey={auraKey} setAuraKey={setAuraKey}
            showAura={showAura} setShowAura={setShowAura}
            powerChoice={powerChoice} setPowerChoice={setPowerChoice}
            powerKey={powerKey} setPowerKey={setPowerKey}
            showPower={showPower} setShowPower={setShowPower}
            powerBase={powerBase} setPowerBase={setPowerBase}
            powerModel={powerModel} setPowerModel={setPowerModel}
            verify={verify} setVerify={setVerify}
            doVerify={doVerify} doSave={doSave} doSkip={doSkip}
            envGroqDetected={envGroqDetected}
            envGeminiDetected={envGeminiDetected}
            envOpenaiDetected={envOpenaiDetected}
          />
        ) : (
          <DwellingRite
            t={t}
            scene={scene} setScene={setScene}
            summonerName={summonerName} setSummonerName={setSummonerName}
            auraKey={auraKey} setAuraKey={setAuraKey}
            showAura={showAura} setShowAura={setShowAura}
            powerChoice={powerChoice} setPowerChoice={setPowerChoice}
            powerKey={powerKey} setPowerKey={setPowerKey}
            showPower={showPower} setShowPower={setShowPower}
            powerBase={powerBase} setPowerBase={setPowerBase}
            powerModel={powerModel} setPowerModel={setPowerModel}
            verify={verify} setVerify={setVerify}
            doVerify={doVerify} doSave={doSave} doSkip={doSkip}
            envGroqDetected={envGroqDetected}
            envGeminiDetected={envGeminiDetected}
            envOpenaiDetected={envOpenaiDetected}
            onSwitchToDirect={() => setMode("direct")}
          />
        )}
      </div>
    </div>
  );
}

// ─── 音樂 visualizer ────────────────────────────────────────

function AudioVisualizer({ muted }: { muted: boolean }) {
  const containerRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (muted) {
      const c = containerRef.current;
      if (c) c.querySelectorAll<HTMLElement>(".eq-wide-bar").forEach((b) => {
        b.style.transform = "scaleY(0.08)";
      });
      return;
    }
    let raf = 0;
    const ACTIVE_BINS = 32;
    const tick = () => {
      const analyser = ritualAudio.getAnalyser();
      const c = containerRef.current;
      if (analyser && c) {
        const data = new Uint8Array(analyser.frequencyBinCount);
        analyser.getByteFrequencyData(data);
        const bars = c.querySelectorAll<HTMLElement>(".eq-wide-bar");
        bars.forEach((bar, i) => {
          const sourceBin = (i * 17) % ACTIVE_BINS;
          const v = data[sourceBin] || 0;
          const scale = 0.1 + (v / 255) * 0.85;
          bar.style.transform = `scaleY(${scale})`;
        });
      }
      raf = requestAnimationFrame(tick);
    };
    tick();
    return () => cancelAnimationFrame(raf);
  }, [muted]);

  return (
    <div ref={containerRef} className="mori-quickstart-header-eq">
      {Array.from({ length: 32 }).map((_, i) => (
        <div key={i} className="eq-wide-bar" />
      ))}
    </div>
  );
}

// ─── 共用 props ────────────────────────────────────────────

interface CommonProps {
  t: (k: string, opts?: any) => string;
  summonerName: string; setSummonerName: (s: string) => void;
  auraKey: string; setAuraKey: (s: string) => void;
  showAura: boolean; setShowAura: (b: boolean) => void;
  powerChoice: PowerChoice; setPowerChoice: (p: PowerChoice) => void;
  powerKey: string; setPowerKey: (s: string) => void;
  showPower: boolean; setShowPower: (b: boolean) => void;
  powerBase: string; setPowerBase: (s: string) => void;
  powerModel: string; setPowerModel: (s: string) => void;
  verify: VerifyState; setVerify: (v: VerifyState) => void;
  doVerify: () => void; doSave: () => void; doSkip: () => void;
  envGroqDetected: boolean;
  envGeminiDetected: boolean;
  envOpenaiDetected: boolean;
}

// ─── 直接模式 ───────────────────────────────────────────────

function DirectForm(props: CommonProps) {
  const { t, summonerName, setSummonerName, auraKey, setAuraKey, showAura, setShowAura,
    powerChoice, setPowerChoice, powerKey, setPowerKey, showPower, setShowPower,
    powerBase, setPowerBase, powerModel, setPowerModel,
    verify, setVerify, doVerify, doSave, doSkip,
    envGroqDetected, envGeminiDetected, envOpenaiDetected } = props;

  const auraReal = auraKey.trim().length > 5 || (envGroqDetected && auraKey.trim() === "");
  // env-only path:env 偵測到 + 沒填 key 也算 ready(custom 多要求 api_base 已填)
  const geminiEnvOnly = powerChoice === "gemini" && envGeminiDetected && powerKey.trim() === "";
  const customEnvOnly = powerChoice === "custom" && envOpenaiDetected && powerKey.trim() === ""
    && powerBase.trim().length > 0;
  const powerReal = powerChoice === "skip"
    || geminiEnvOnly
    || customEnvOnly
    || (powerChoice !== null && powerKey.trim().length > 5);
  const nameReal = summonerName.trim().length > 0;
  const canVerify = nameReal && auraReal && powerReal && verify.kind !== "running";
  const canSave = nameReal && auraReal && powerReal && (
    verify.kind === "ok"
    || (envGroqDetected && powerChoice === "skip")
    || geminiEnvOnly
    || customEnvOnly
  );

  return (
    <div className="mori-quickstart-ritual-step mori-direct-form">
      <div className="mori-quickstart-scene-content">
      <p className="mori-quickstart-intro">{t("quickstart.direct_intro")}</p>

      <div className="mori-quickstart-field">
        <label>{t("quickstart.direct_name_label")}</label>
        <input
          type="text"
          className="mori-input"
          placeholder={t("quickstart.direct_name_placeholder")}
          value={summonerName}
          onChange={(e) => setSummonerName(e.target.value)}
        />
      </div>

      <div className="mori-quickstart-field">
        <label>
          {t("quickstart.dwelling_scene_2_aura_tech_label")}
          <a
            href={GROQ_HELP_URL}
            onClick={(e) => { e.preventDefault(); invoke("open_external_url", { url: GROQ_HELP_URL }).catch(console.warn); }}
            className="mori-quickstart-help-link"
          >
            {t("quickstart.direct_groq_help")}
          </a>
        </label>
        {envGroqDetected && (
          <div className="mori-quickstart-env-banner">
            ✓ {t("quickstart.direct_groq_env_detected")}
          </div>
        )}
        <div className="mori-quickstart-key-input-row">
          <input
            type={showAura ? "text" : "password"}
            className="mori-input"
            placeholder={t("quickstart.dwelling_scene_2_aura_placeholder")}
            value={auraKey}
            onChange={(e) => {
              setAuraKey(e.target.value);
              if (verify.kind !== "idle") setVerify({ kind: "idle" });
            }}
          />
          <button type="button" className="mori-btn ghost small" onClick={() => setShowAura(!showAura)}>
            {showAura ? t("quickstart.dwelling_scene_2_hide_key") : t("quickstart.dwelling_scene_2_show_key")}
          </button>
        </div>
      </div>

      <div className="mori-quickstart-field">
        <label>{t("quickstart.choose_provider")}</label>
        <div className="mori-quickstart-provider-grid">
          {(["gemini", "custom", "skip"] as const).map((p) => (
            <button
              key={p}
              className={`mori-quickstart-provider-card ${powerChoice === p ? "active" : ""}`}
              onClick={() => { setPowerChoice(p); if (verify.kind !== "idle") setVerify({ kind: "idle" }); }}
            >
              <span className="provider-name">
                {p === "gemini" ? t("quickstart.dwelling_scene_3_card_gemini_name")
                  : p === "custom" ? t("quickstart.dwelling_scene_3_card_compat_name")
                  : t("quickstart.direct_skip_link")}
              </span>
              <span className="provider-hint">
                {p === "gemini" ? t("quickstart.dwelling_scene_3_card_gemini_hint")
                  : p === "custom" ? t("quickstart.dwelling_scene_3_card_compat_hint")
                  : t("quickstart.direct_skip_note")}
              </span>
            </button>
          ))}
        </div>
      </div>

      {powerChoice === "custom" && (
        <>
          <div className="mori-quickstart-field">
            <label>{t("quickstart.dwelling_scene_3_api_base_label")}</label>
            <input
              type="text"
              className="mori-input"
              placeholder={t("quickstart.dwelling_scene_3_api_base_placeholder")}
              value={powerBase}
              onChange={(e) => { setPowerBase(e.target.value); if (verify.kind !== "idle") setVerify({ kind: "idle" }); }}
            />
          </div>
          <div className="mori-quickstart-field">
            <label>{t("quickstart.dwelling_scene_3_model_label")}</label>
            <input
              type="text"
              className="mori-input"
              placeholder={t("quickstart.dwelling_scene_3_model_placeholder")}
              value={powerModel}
              onChange={(e) => setPowerModel(e.target.value)}
            />
          </div>
        </>
      )}

      {powerChoice && powerChoice !== "skip" && (
        <div className="mori-quickstart-field">
          <label>
            {t("quickstart.dwelling_scene_3_power_tech_label")}
            <a
              href={powerChoice === "gemini" ? GEMINI_HELP_URL : "#"}
              onClick={(e) => {
                e.preventDefault();
                if (powerChoice === "gemini") {
                  invoke("open_external_url", { url: GEMINI_HELP_URL }).catch(console.warn);
                }
              }}
              className="mori-quickstart-help-link"
            >
              {t("quickstart.direct_provider_help")}
            </a>
          </label>
          {((powerChoice === "gemini" && envGeminiDetected) ||
            (powerChoice === "custom" && envOpenaiDetected)) && (
            <div className="mori-quickstart-env-banner">
              ✓ {t(powerChoice === "gemini"
                ? "quickstart.scene_3_env_detected_gemini"
                : "quickstart.scene_3_env_detected_openai")}
            </div>
          )}
          <div className="mori-quickstart-key-input-row">
            <input
              type={showPower ? "text" : "password"}
              className="mori-input"
              placeholder={t("quickstart.dwelling_scene_3_power_placeholder")}
              value={powerKey}
              onChange={(e) => { setPowerKey(e.target.value); if (verify.kind !== "idle") setVerify({ kind: "idle" }); }}
            />
            <button type="button" className="mori-btn ghost small" onClick={() => setShowPower(!showPower)}>
              {showPower ? t("quickstart.dwelling_scene_2_hide_key") : t("quickstart.dwelling_scene_2_show_key")}
            </button>
          </div>
        </div>
      )}

      <div className="mori-quickstart-verify-row">
        <button className="mori-btn" onClick={doVerify} disabled={!canVerify}>
          {verify.kind === "running" ? t("quickstart.verifying") : t("quickstart.verify_button")}
        </button>
        {verify.kind === "ok" && <span className="mori-quickstart-verify-msg ok">✓ {verify.msg}</span>}
        {verify.kind === "err" && <span className="mori-quickstart-verify-msg err">✗ {verify.msg}</span>}
      </div>
      </div>

      <div className="mori-quickstart-footer">
        <button className="mori-btn" onClick={doSkip}>{t("quickstart.direct_skip_button")}</button>
        <button className="mori-btn primary" onClick={doSave} disabled={!canSave}>{t("quickstart.direct_save_button")}</button>
      </div>
    </div>
  );
}

// ─── 宿靈儀式 · 5 幕 ─────────────────────────────────────────

interface DwellingProps extends CommonProps {
  scene: number;
  setScene: (s: number) => void;
  onSwitchToDirect: () => void;
}

function DwellingRite(props: DwellingProps) {
  const { scene, setScene } = props;
  const total = 5;

  const dots = (
    <div className="mori-quickstart-ritual-progress">
      {Array.from({ length: total }).map((_, i) => (
        <span
          key={i}
          className={`step-dot ${i + 1 === scene ? "active" : ""} ${i + 1 < scene ? "done" : ""}`}
        />
      ))}
    </div>
  );

  return (
    <div className="mori-quickstart-ritual">
      {scene === 1 && <SceneSummoning {...props} dots={dots} onNext={() => setScene(2)} />}
      {scene === 2 && <SceneAura {...props} dots={dots} onBack={() => setScene(1)} onNext={() => setScene(3)} />}
      {scene === 3 && <ScenePower {...props} dots={dots} onBack={() => setScene(2)} onNext={() => setScene(4)} />}
      {scene === 4 && <SceneSealing {...props} dots={dots} onBack={() => setScene(3)} onNext={() => setScene(5)} />}
      {scene === 5 && <SceneSettling {...props} dots={dots} />}
    </div>
  );
}

type StepProps = DwellingProps & {
  dots: React.ReactNode;
  onBack?: () => void;
  onNext?: () => void;
};

// ─── 第一幕 · 召喚 ──────────────────────────────────────────

function SceneSummoning({
  t, summonerName, setSummonerName, dots, onNext, doSkip, onSwitchToDirect,
}: StepProps) {
  const nameReady = summonerName.trim().length > 0;
  // 召喚師打字時 confirm 兩段會動態冒出來,scroll 容器自動拉到底,
  // 不然 user 看不到那段確認語
  const contentRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (nameReady && contentRef.current) {
      contentRef.current.scrollTop = contentRef.current.scrollHeight;
    }
  }, [nameReady, summonerName]);
  return (
    <div className="mori-quickstart-ritual-step">
      <div ref={contentRef} className="mori-quickstart-scene-content">
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_1_intro")}</p>
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_1_descent")}</p>
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_1_arrival")}</p>
        <p className="mori-ritual-narrative whisper">{t("quickstart.dwelling_scene_1_say_1")}</p>
        <p className="mori-ritual-narrative whisper">{t("quickstart.dwelling_scene_1_say_2")}</p>
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_1_action_3")}</p>
        <p className="mori-ritual-narrative whisper">{t("quickstart.dwelling_scene_1_say_3")}</p>

        <div className="mori-quickstart-field">
          <label>{t("quickstart.dwelling_scene_1_name_label")}</label>
          <input
            type="text"
            className="mori-input"
            placeholder={t("quickstart.dwelling_scene_1_name_placeholder")}
            value={summonerName}
            onChange={(e) => setSummonerName(e.target.value)}
            autoFocus
          />
        </div>

        {nameReady && (
          <>
            <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_1_confirm_1")}</p>
            <p className="mori-ritual-narrative whisper">
              {t("quickstart.dwelling_scene_1_confirm_2", { name: summonerName.trim() })}
            </p>
          </>
        )}
      </div>

      {dots}
      <div className="mori-quickstart-footer">
        <button className="mori-btn ghost" onClick={doSkip}>{t("quickstart.ritual_dismiss")}</button>
        <button className="mori-btn ghost" onClick={onSwitchToDirect}>{t("quickstart.ritual_switch_direct")}</button>
        <button className="mori-btn primary" onClick={onNext} disabled={!nameReady}>
          {nameReady
            ? t("quickstart.dwelling_scene_1_button", { name: summonerName.trim() })
            : t("quickstart.dwelling_scene_1_name_label")}
        </button>
      </div>
    </div>
  );
}

// ─── 第二幕 · 靈氣 ──────────────────────────────────────────

function SceneAura({
  t, summonerName, auraKey, setAuraKey, showAura, setShowAura,
  setVerify, envGroqDetected, dots, onBack, onNext,
}: StepProps) {
  const auraReady = auraKey.trim().length > 5 || (envGroqDetected && auraKey.trim() === "");

  return (
    <div className="mori-quickstart-ritual-step">
      <div className="mori-quickstart-scene-content">
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_2_intro")}</p>
        <p className="mori-ritual-narrative whisper">
          {t("quickstart.dwelling_scene_2_say_1", { name: summonerName.trim() })}
        </p>
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_2_intro_2")}</p>
        <p className="mori-ritual-narrative whisper">{t("quickstart.dwelling_scene_2_say_2")}</p>

        <div className="mori-quickstart-field">
          <label>
            {t("quickstart.dwelling_scene_2_aura_tech_label")}
            <a
              href={GROQ_HELP_URL}
              onClick={(e) => { e.preventDefault(); invoke("open_external_url", { url: GROQ_HELP_URL }).catch(console.warn); }}
              className="mori-quickstart-help-link"
            >
              {t("quickstart.dwelling_scene_2_aura_help")} ↗
            </a>
          </label>
          {envGroqDetected && (
            <div className="mori-quickstart-env-banner">
              ✓ {t("quickstart.dwelling_scene_2_env_detected")}
            </div>
          )}
          <div className="mori-quickstart-key-input-row">
            <input
              type={showAura ? "text" : "password"}
              className="mori-input"
              placeholder={t("quickstart.dwelling_scene_2_aura_placeholder")}
              value={auraKey}
              onChange={(e) => {
                setAuraKey(e.target.value);
                setVerify({ kind: "idle" });
              }}
              autoFocus
            />
            <button type="button" className="mori-btn ghost small" onClick={() => setShowAura(!showAura)}>
              {showAura ? t("quickstart.dwelling_scene_2_hide_key") : t("quickstart.dwelling_scene_2_show_key")}
            </button>
          </div>
        </div>
      </div>

      {dots}
      <div className="mori-quickstart-footer">
        <button className="mori-btn" onClick={onBack}>{t("quickstart.ritual_back")}</button>
        <button className="mori-btn primary" onClick={onNext} disabled={!auraReady}>
          {t("quickstart.dwelling_scene_2_button")}
        </button>
      </div>
    </div>
  );
}

// ─── 第三幕 · 靈力 ──────────────────────────────────────────

function ScenePower({
  t, summonerName, powerChoice, setPowerChoice,
  powerKey, setPowerKey, showPower, setShowPower,
  powerBase, setPowerBase, powerModel, setPowerModel,
  setVerify, envGeminiDetected, envOpenaiDetected, dots, onBack, onNext,
}: StepProps) {
  // env-only path:env 偵測到 + 沒填 key 也算 ready(custom 多要求 api_base)
  const geminiEnvOnly =
    powerChoice === "gemini" && envGeminiDetected && powerKey.trim() === "";
  const customEnvOnly =
    powerChoice === "custom" && envOpenaiDetected && powerKey.trim() === "" &&
    powerBase.trim().length > 5;
  const keyReady =
    powerChoice === "skip" ||
    geminiEnvOnly ||
    customEnvOnly ||
    (powerChoice !== null && powerKey.trim().length > 5 &&
      (powerChoice === "gemini" || powerBase.trim().length > 5));

  return (
    <div className="mori-quickstart-ritual-step">
      <div className="mori-quickstart-scene-content">
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_3_intro_1")}</p>
        <p className="mori-ritual-narrative whisper">
          {t("quickstart.dwelling_scene_3_say_thanks", { name: summonerName.trim() })}
        </p>
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_3_intro_2")}</p>
        <p className="mori-ritual-narrative whisper">{t("quickstart.dwelling_scene_3_say_1")}</p>
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_3_intro_3")}</p>
        <p className="mori-ritual-narrative whisper">{t("quickstart.dwelling_scene_3_say_2")}</p>
        <p className="mori-ritual-narrative whisper">{t("quickstart.dwelling_scene_3_say_3")}</p>

        <div className="mori-quickstart-provider-grid">
        {(["gemini", "custom"] as const).map((p) => (
          <button
            key={p}
            className={`mori-quickstart-provider-card ${powerChoice === p ? "active" : ""}`}
            onClick={() => {
              setPowerChoice(p);
              setVerify({ kind: "idle" });
            }}
          >
            <span className="provider-name">
              {p === "gemini"
                ? `✦ ${t("quickstart.dwelling_scene_3_card_gemini_name")}`
                : `❂ ${t("quickstart.dwelling_scene_3_card_compat_name")}`}
            </span>
            <span className="provider-hint">
              {p === "gemini"
                ? t("quickstart.dwelling_scene_3_card_gemini_hint")
                : t("quickstart.dwelling_scene_3_card_compat_hint")}
            </span>
          </button>
        ))}
      </div>

      {powerChoice === "custom" && (
        <>
          <div className="mori-quickstart-field">
            <label>{t("quickstart.dwelling_scene_3_api_base_label")}</label>
            <input
              type="text"
              className="mori-input"
              placeholder={t("quickstart.dwelling_scene_3_api_base_placeholder")}
              value={powerBase}
              onChange={(e) => {
                setPowerBase(e.target.value);
                setVerify({ kind: "idle" });
              }}
            />
          </div>
          <div className="mori-quickstart-field">
            <label>{t("quickstart.dwelling_scene_3_model_label")}</label>
            <input
              type="text"
              className="mori-input"
              placeholder={t("quickstart.dwelling_scene_3_model_placeholder")}
              value={powerModel}
              onChange={(e) => setPowerModel(e.target.value)}
            />
          </div>
        </>
      )}

      {powerChoice && powerChoice !== "skip" && (
        <div className="mori-quickstart-field">
          <label>
            {t("quickstart.dwelling_scene_3_power_tech_label")}
            <a
              href={powerChoice === "gemini" ? GEMINI_HELP_URL : "#"}
              onClick={(e) => {
                e.preventDefault();
                if (powerChoice === "gemini") {
                  invoke("open_external_url", { url: GEMINI_HELP_URL }).catch(console.warn);
                }
              }}
              className="mori-quickstart-help-link"
            >
              {t("quickstart.dwelling_scene_3_power_help")}
            </a>
          </label>
          {((powerChoice === "gemini" && envGeminiDetected) ||
            (powerChoice === "custom" && envOpenaiDetected)) && (
            <div className="mori-quickstart-env-banner">
              ✓ {t(powerChoice === "gemini"
                ? "quickstart.scene_3_env_detected_gemini"
                : "quickstart.scene_3_env_detected_openai")}
            </div>
          )}
          <div className="mori-quickstart-key-input-row">
            <input
              type={showPower ? "text" : "password"}
              className="mori-input"
              placeholder={t("quickstart.dwelling_scene_3_power_placeholder")}
              value={powerKey}
              onChange={(e) => {
                setPowerKey(e.target.value);
                setVerify({ kind: "idle" });
              }}
              autoFocus
            />
            <button type="button" className="mori-btn ghost small" onClick={() => setShowPower(!showPower)}>
              {showPower ? t("quickstart.dwelling_scene_2_hide_key") : t("quickstart.dwelling_scene_2_show_key")}
            </button>
          </div>
        </div>
      )}

      {/* Skip link — 視覺上比兩張卡更低調 */}
      <button
        className="mori-quickstart-skip-link"
        onClick={() => {
          setPowerChoice("skip");
          setVerify({ kind: "idle" });
        }}
      >
        {powerChoice === "skip" ? "✓ " : "─ "}
        {t("quickstart.dwelling_scene_3_skip_link")}
        {" ─"}
      </button>
      {powerChoice === "skip" && (
        <p className="mori-quickstart-skip-note">{t("quickstart.dwelling_scene_3_skip_note")}</p>
      )}
      </div>

      {dots}
      <div className="mori-quickstart-footer">
        <button className="mori-btn" onClick={onBack}>{t("quickstart.ritual_back")}</button>
        <button className="mori-btn primary" onClick={onNext} disabled={!keyReady}>
          {powerChoice === "skip"
            ? t("quickstart.dwelling_scene_3_button_skip")
            : t("quickstart.dwelling_scene_3_button")}
        </button>
      </div>
    </div>
  );
}

// ─── 第四幕 · 驗印 ──────────────────────────────────────────

function SceneSealing({
  t, summonerName, verify, powerChoice, doVerify, setScene, dots, onBack, onNext,
}: StepProps) {
  // 進入這幕一定重驗 — 即使 prefill 把 verify 設成 ok,也要重跑一次儀式
  // (key 可能已過期 / revoked,而且 user 應該看到「⋯⋯順著脈絡⋯⋯」這段)
  useEffect(() => {
    doVerify();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 累積顯示:每階段是新一段,不蓋掉前一段。
  // - 靈氣這段:running.phase==="aura" 或之後任何狀態 都顯示;phase==="aura" pulse
  // - 靈力這段:auraVerified 或之後 顯示;phase==="power" pulse;skip 不顯示
  // - 對上了:verify.kind==="ok"
  // - err:依 which 顯示對應 phase 的失敗訊息(取代該 phase 的 pulse)
  const showAura =
    verify.kind === "running" ||
    verify.kind === "ok" ||
    (verify.kind === "err" && verify.which === "aura");
  const auraPulsing = verify.kind === "running" && verify.phase === "aura";
  const auraFailed = verify.kind === "err" && verify.which === "aura";

  const showPower =
    powerChoice !== "skip" &&
    ((verify.kind === "running" && verify.auraVerified) ||
      verify.kind === "ok" ||
      (verify.kind === "err" && verify.which === "power"));
  const powerPulsing = verify.kind === "running" && verify.phase === "power";
  const powerFailed = verify.kind === "err" && verify.which === "power";

  return (
    <div className="mori-quickstart-ritual-step">
      <div className="mori-quickstart-scene-content">
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_4_intro")}</p>

        {/* 第一階段敘述:靈氣的脈絡 */}
        {showAura && !auraFailed && (
          <p className={`mori-ritual-narrative whisper ${auraPulsing ? "mori-ritual-feeling-pulse" : ""}`}>
            {t("quickstart.dwelling_scene_4_feeling_aura")}
          </p>
        )}
        {auraFailed && (
          <>
            <p className="mori-ritual-narrative whisper">
              {t("quickstart.dwelling_scene_4_no_match_aura")}
            </p>
            <p className="mori-quickstart-verify-error-detail">{verify.msg}</p>
          </>
        )}

        {/* 第二階段敘述:靈力的脈絡 — 加 whisper-late 緩淡入 */}
        {showPower && !powerFailed && (
          <p className={`mori-ritual-narrative whisper whisper-late ${powerPulsing ? "mori-ritual-feeling-pulse" : ""}`}>
            {t("quickstart.dwelling_scene_4_feeling_power")}
          </p>
        )}
        {powerFailed && (
          <>
            <p className="mori-ritual-narrative whisper">
              {t("quickstart.dwelling_scene_4_no_match_power")}
            </p>
            <p className="mori-quickstart-verify-error-detail">{verify.msg}</p>
          </>
        )}

        {/* 最後揭曉:對上了 — 緩慢淡入 2.5s,儀式氣氛不破 */}
        {verify.kind === "ok" && (
          <p className="mori-ritual-narrative whisper whisper-final">
            {t("quickstart.dwelling_scene_4_match", { name: summonerName.trim() })}
          </p>
        )}
      </div>

      {dots}
      {/* err 時 footer 只顯示「回去重填」按鈕 — 同 key 再試必失敗,直接送回對應幕 */}
      {verify.kind === "err" ? (
        <div className="mori-quickstart-footer single">
          <button
            className="mori-btn primary"
            onClick={() => setScene(verify.which === "aura" ? 2 : 3)}
          >
            {verify.which === "aura"
              ? t("quickstart.dwelling_scene_4_err_back_aura")
              : t("quickstart.dwelling_scene_4_err_back_power")}
          </button>
        </div>
      ) : (
      <div className="mori-quickstart-footer">
        <button className="mori-btn" onClick={onBack}>{t("quickstart.ritual_back")}</button>
        <button className="mori-btn primary" onClick={onNext} disabled={verify.kind !== "ok"}>
          {t("quickstart.dwelling_scene_4_button")}
        </button>
      </div>
      )}
    </div>
  );
}

// ─── 第五幕 · 安頓 ──────────────────────────────────────────

function SceneSettling({
  t, summonerName, doSave,
}: StepProps) {
  return (
    <div className="mori-quickstart-ritual-step settling">
      <div className="mori-quickstart-scene-content">
        <p className="mori-ritual-narrative whisper">{t("quickstart.dwelling_scene_5_silence")}</p>
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_5_opening")}</p>
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_5_revival")}</p>
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_5_settle_1")}</p>
        <p className="mori-ritual-narrative whisper">{t("quickstart.dwelling_scene_5_settle_2")}</p>
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_5_look")}</p>
        <p className="mori-ritual-narrative whisper">
          {t("quickstart.dwelling_scene_5_promise_1", { name: summonerName.trim() })}
        </p>
        <p className="mori-ritual-narrative whisper">{t("quickstart.dwelling_scene_5_promise_2")}</p>
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_5_book_action")}</p>
        <p className="mori-ritual-narrative whisper">{t("quickstart.dwelling_scene_5_book_name")}</p>
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_5_world_alive")}</p>
        <p className="mori-ritual-narrative">{t("quickstart.dwelling_scene_5_breathing")}</p>
        <p className="mori-ritual-narrative aftermath">{t("quickstart.dwelling_scene_5_sprout")}</p>
        <p className="mori-ritual-narrative whisper">
          {t("quickstart.dwelling_scene_5_thanks", { name: summonerName.trim() })}
        </p>
      </div>

      <div className="mori-quickstart-footer single">
        <button className="mori-btn primary large" onClick={doSave}>
          {t("quickstart.dwelling_scene_5_button")}
        </button>
      </div>
    </div>
  );
}

// ─── First-run detection ─────────────────────────────────────

/**
 * 偵測「需要顯示宿靈儀式」 — 純看 flag:
 *   config.json `quickstart_completed === true` → 不顯示
 *   否則 → 顯示(第一次跑就會跳)
 */
export async function shouldShowQuickstart(): Promise<boolean> {
  try {
    const raw = await invoke<string>("config_read");
    const cfg = JSON.parse(raw);
    return cfg.quickstart_completed !== true;
  } catch (e) {
    console.warn("[quickstart] shouldShow check failed, defaulting to false", e);
    return false;
  }
}

/** 共用 helper:寫 config.json 把 quickstart_completed flag 立起來。 */
export async function markQuickstartCompleted(): Promise<void> {
  let cfg: any = {};
  try {
    const raw = await invoke<string>("config_read");
    cfg = JSON.parse(raw);
  } catch {
    cfg = {};
  }
  cfg.quickstart_completed = true;
  delete cfg.quickstart_skipped;
  await invoke("config_write", { text: JSON.stringify(cfg, null, 2) });
}
