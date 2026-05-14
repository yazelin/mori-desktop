// F — Quickstart onboarding modal。
//
// 第一次跑 mori-desktop 沒設任何 LLM API key 時自動跳出。兩種模式可切換:
//
// **直接模式**(plain):單頁表單,選 provider + 貼 key + 驗證 + 存。最快路徑。
// **儀式模式**(ritual):5 步沉浸式入林流程,Mori 詩意自述帶你走過。較慢、較有儀式感。
//
// 對應 world-tree lore/the-forest.md:「取名是儀式,不是命名變數。」
// 對應 ONBOARDING.md:「每個走入森林的人,遲早會遇見一個願意為他停下來的精靈。」

import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { IconClose, IconGlobe, IconSun, IconMoon, IconEqualizer } from "./icons";
import { setLocale, nextLocale } from "./i18n";
import { toggleTheme, loadActiveTheme } from "./theme";
import { ritualAudio } from "./ritualAudio";

type Provider = "groq" | "openai_compat";
type Mode = "direct" | "ritual";

type VerifyState =
  | { kind: "idle" }
  | { kind: "verifying" }
  | { kind: "ok"; msg: string }
  | { kind: "err"; msg: string };

const PROVIDER_INFO: Record<Provider, {
  label: string;
  helpUrl: string;
  placeholder: string;
  defaultBase?: string;
  defaultModel?: string;
}> = {
  groq: {
    label: "Groq",
    helpUrl: "https://console.groq.com/keys",
    placeholder: "gsk_...",
  },
  openai_compat: {
    label: "OpenAI-相容",
    helpUrl: "https://aistudio.google.com/app/apikey",
    placeholder: "sk-... / AIzaSy... / gsk_...",
    defaultBase: "https://generativelanguage.googleapis.com/v1beta/openai",
    defaultModel: "gemini-2.5-flash",
  },
};

interface QuickstartProps {
  onDone: () => void;
}

export function Quickstart({ onDone }: QuickstartProps) {
  const { t, i18n } = useTranslation();
  const [mode, setMode] = useState<Mode>("ritual");
  const [provider, setProvider] = useState<Provider>("groq");
  const [keyText, setKeyText] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [verify, setVerify] = useState<VerifyState>({ kind: "idle" });
  // OpenAI-相容專用:api_base + model(預設 Gemini OpenAI endpoint)
  const [apiBase, setApiBase] = useState(PROVIDER_INFO.openai_compat.defaultBase ?? "");
  const [model, setModel] = useState(PROVIDER_INFO.openai_compat.defaultModel ?? "");
  // 選 openai_compat 為 LLM 時的可選「Groq key 給 STT」欄位
  const [sttKeyText, setSttKeyText] = useState("");
  // 偵測 GROQ_API_KEY env var 是否已設(後端 startup 會讀進 state.groq_api_key)
  // 若 true → user 不用再貼 key,只要 Save 就能用環境變數的
  const [envGroqDetected, setEnvGroqDetected] = useState(false);
  useEffect(() => {
    invoke<boolean>("has_groq_key").then(setEnvGroqDetected).catch(() => {});
  }, []);

  // 儀式模式 → 開 ambient(/audio/ritual-ambient.mp3),切走 / 關 modal 時停。
  // 預設不靜音 — 有真音檔了,自動播給儀式感氛圍。user 不想聽自己 toggle 關。
  const [audioMuted, setAudioMuted] = useState(false);
  // Quickstart 內也能切 theme(因 user 可能 onboarding 時想換明暗)
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
    if (mode === "ritual" && !audioMuted) {
      ritualAudio.startAmbient();
    } else {
      ritualAudio.stopAmbient();
    }
    return () => { ritualAudio.stopAmbient(); };
  }, [mode, audioMuted]);
  // 儀式模式:當前步驟 0..4
  const [ritualStep, setRitualStep] = useState(0);

  useEffect(() => {
    setKeyText("");
    setVerify({ kind: "idle" });
  }, [provider]);

  const info = PROVIDER_INFO[provider];
  const canVerify = keyText.trim().length > 5 && verify.kind !== "verifying";
  // 環境變數已偵測 + 選 groq → 不用驗證 / 貼 key 也能存
  const envOnlyMode = envGroqDetected && provider === "groq" && keyText.trim() === "";
  const canSave = envOnlyMode || verify.kind === "ok";

  const doVerify = async () => {
    setVerify({ kind: "verifying" });
    try {
      const args: any = { provider, key: keyText.trim() };
      if (provider === "openai_compat") args.apiBase = apiBase.trim();
      const msg = await invoke<string>("verify_llm_key", args);
      setVerify({ kind: "ok", msg });
    } catch (e: any) {
      setVerify({ kind: "err", msg: String(e) });
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

      if (provider === "groq") {
        // Groq:單純 key,base 走預設(groq.com)
        if (!cfg.providers.groq) cfg.providers.groq = {};
        // 若 user 沒貼 key 但 env 偵測到 → 不寫 api_key,讓後端 discover_api_key
        // 從 GROQ_API_KEY env 拿;若有貼就 overrides env(user 明確意圖)
        const k = keyText.trim();
        if (k) {
          cfg.providers.groq.api_key = k;
        }
        cfg.provider = "groq";
        cfg.stt_provider = "groq";
      } else {
        // openai_compat:寫進 providers.openai_compat,含 api_base + api_key + model
        if (!cfg.providers.openai_compat) cfg.providers.openai_compat = {};
        cfg.providers.openai_compat.api_base = apiBase.trim();
        cfg.providers.openai_compat.api_key = keyText.trim();
        if (model.trim()) cfg.providers.openai_compat.model = model.trim();
        cfg.provider = "openai_compat";
        // STT:user 有貼 Groq key 走 groq,沒貼走本機 Whisper
        const sttKey = sttKeyText.trim();
        if (sttKey) {
          if (!cfg.providers.groq) cfg.providers.groq = {};
          cfg.providers.groq.api_key = sttKey;
          cfg.stt_provider = "groq";
        } else {
          cfg.stt_provider = "whisper-local";
        }
      }
      cfg.quickstart_completed = true;
      delete cfg.quickstart_skipped;
      await invoke("config_write", { text: JSON.stringify(cfg, null, 2) });
      // 儀式模式:走到最後一步「甦醒」再 onDone
      if (mode === "ritual") {
        setRitualStep(4);
        setTimeout(onDone, 1500);
      } else {
        onDone();
      }
    } catch (e: any) {
      setVerify({ kind: "err", msg: `存設定失敗:${e}` });
    }
  };

  const doSkip = async () => {
    try {
      await markQuickstartCompleted();
    } catch (e) {
      console.warn("[quickstart] failed to mark completed", e);
    }
    onDone();
  };

  return (
    <div className={`mori-quickstart-backdrop mode-${mode}`}>
      <div className="mori-quickstart-modal" role="dialog" aria-modal="true">
        <div className="mori-quickstart-header">
          <h2>{mode === "ritual" ? t("quickstart.ritual_title") : t("quickstart.title")}</h2>
          <div className="mori-quickstart-mode-toggle">
            <button
              className={`mori-quickstart-mode-btn ${mode === "direct" ? "active" : ""}`}
              onClick={() => setMode("direct")}
              title={t("quickstart.mode_direct_hint")}
            >{t("quickstart.mode_direct")}</button>
            <button
              className={`mori-quickstart-mode-btn ${mode === "ritual" ? "active" : ""}`}
              onClick={() => { setMode("ritual"); setRitualStep(0); }}
              title={t("quickstart.mode_ritual_hint")}
            >{t("quickstart.mode_ritual")}</button>
          </div>
          <button
            className="mori-btn ghost"
            onClick={() => {
              const next = nextLocale(i18n.language);
              setLocale(next).catch((e) => console.error("[i18n] toggle failed", e));
            }}
            title={i18n.language === "zh-TW" ? "Switch to English" : "切到繁體中文"}
          >
            <IconGlobe width={14} height={14} />
            <span style={{ marginLeft: 4, fontSize: 11 }}>
              {i18n.language === "zh-TW" ? "EN" : "繁中"}
            </span>
          </button>
          <button
            className="mori-btn ghost"
            onClick={handleThemeToggle}
            title={themeBase === "dark" ? t("common.settings") : t("common.settings")}
          >
            {themeBase === "dark" ? <IconSun width={14} height={14} /> : <IconMoon width={14} height={14} />}
            <span style={{ marginLeft: 4, fontSize: 11 }}>
              {themeBase === "dark" ? "Light" : "Dark"}
            </span>
          </button>
          {mode === "ritual" && (
            <button
              className="mori-btn ghost"
              onClick={() => setAudioMuted(!audioMuted)}
              title={audioMuted ? t("quickstart.audio_unmute") : t("quickstart.audio_mute")}
            >
              <IconEqualizer width={16} height={16} playing={!audioMuted} />
            </button>
          )}
          <button className="mori-btn ghost" onClick={doSkip} title={t("quickstart.skip_title")}>
            <IconClose width={14} height={14} />
          </button>
        </div>

        {/* 儀式模式:header 下方音樂 visualizer(真讀 freq data,靜音時扁平) */}
        {mode === "ritual" && <AudioVisualizer muted={audioMuted} />}

        {mode === "direct" ? (
          <DirectForm
            t={t}
            provider={provider} setProvider={setProvider}
            keyText={keyText} setKeyText={setKeyText}
            showKey={showKey} setShowKey={setShowKey}
            info={info}
            verify={verify} setVerify={setVerify}
            canVerify={canVerify} canSave={canSave}
            doVerify={doVerify} doSave={doSave} doSkip={doSkip}
            apiBase={apiBase} setApiBase={setApiBase}
            model={model} setModel={setModel}
            sttKeyText={sttKeyText} setSttKeyText={setSttKeyText}
            envGroqDetected={envGroqDetected}
          />
        ) : (
          <RitualFlow
            t={t}
            step={ritualStep} setStep={setRitualStep}
            provider={provider} setProvider={setProvider}
            keyText={keyText} setKeyText={setKeyText}
            showKey={showKey} setShowKey={setShowKey}
            info={info}
            verify={verify} setVerify={setVerify}
            canVerify={canVerify} canSave={canSave}
            doVerify={doVerify} doSave={doSave} doSkip={doSkip}
            apiBase={apiBase} setApiBase={setApiBase}
            model={model} setModel={setModel}
            sttKeyText={sttKeyText} setSttKeyText={setSttKeyText}
            envGroqDetected={envGroqDetected}
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
      // 靜音 → 全部 bar 壓平像 dashed line
      const c = containerRef.current;
      if (c) c.querySelectorAll<HTMLElement>(".eq-wide-bar").forEach((b) => {
        b.style.transform = "scaleY(0.08)";
      });
      return;
    }
    let raf = 0;
    const tick = () => {
      const analyser = ritualAudio.getAnalyser();
      const c = containerRef.current;
      if (analyser && c) {
        const data = new Uint8Array(analyser.frequencyBinCount);
        analyser.getByteFrequencyData(data);
        const bars = c.querySelectorAll<HTMLElement>(".eq-wide-bar");
        bars.forEach((bar, i) => {
          const v = data[i] || 0;
          // scale 0.08 ~ 0.7,音量越大條越高,但天花板控住不誇張
          const scale = 0.08 + (v / 255) * 0.62;
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

// ─── 直接模式 ───────────────────────────────────────────────

interface FormProps {
  t: (k: string) => string;
  provider: Provider;
  setProvider: (p: Provider) => void;
  keyText: string;
  setKeyText: (k: string) => void;
  showKey: boolean;
  setShowKey: (b: boolean) => void;
  info: typeof PROVIDER_INFO[Provider];
  verify: VerifyState;
  setVerify: (v: VerifyState) => void;
  canVerify: boolean;
  canSave: boolean;
  doVerify: () => void;
  doSave: () => void;
  doSkip: () => void;
  apiBase: string;
  setApiBase: (s: string) => void;
  model: string;
  setModel: (s: string) => void;
  sttKeyText: string;
  setSttKeyText: (s: string) => void;
  envGroqDetected: boolean;
}

function DirectForm({
  t, provider, setProvider, keyText, setKeyText, showKey, setShowKey,
  info, verify, setVerify, canVerify, canSave, doVerify, doSave, doSkip,
  apiBase, setApiBase, model, setModel, sttKeyText, setSttKeyText, envGroqDetected,
}: FormProps) {
  return (
    <>
      <p className="mori-quickstart-intro">{t("quickstart.intro")}</p>

      {envGroqDetected && provider === "groq" && (
        <div className="mori-quickstart-env-banner">
          ✓ {t("quickstart.env_detected")}
        </div>
      )}

      <div className="mori-quickstart-field">
        <label>{t("quickstart.choose_provider")}</label>
        <div className="mori-quickstart-provider-grid">
          {(Object.keys(PROVIDER_INFO) as Provider[]).map((p) => (
            <button
              key={p}
              className={`mori-quickstart-provider-card ${provider === p ? "active" : ""}`}
              onClick={() => setProvider(p)}
            >
              <span className="provider-name">{PROVIDER_INFO[p].label}</span>
              <span className="provider-hint">
                {p === "groq" ? t("quickstart.groq_hint") : t("quickstart.gemini_hint")}
              </span>
            </button>
          ))}
        </div>
      </div>

      {provider === "openai_compat" && (
        <>
          <div className="mori-quickstart-field">
            <label>{t("quickstart.api_base_label")}</label>
            <input
              type="text"
              className="mori-input"
              placeholder="https://generativelanguage.googleapis.com/v1beta/openai"
              value={apiBase}
              onChange={(e) => {
                setApiBase(e.target.value);
                if (verify.kind !== "idle") setVerify({ kind: "idle" });
              }}
            />
            <p className="mori-quickstart-field-hint">{t("quickstart.api_base_hint")}</p>
          </div>
          <div className="mori-quickstart-field">
            <label>{t("quickstart.model_label")}</label>
            <input
              type="text"
              className="mori-input"
              placeholder="gemini-2.5-flash / gpt-4o / deepseek-chat"
              value={model}
              onChange={(e) => setModel(e.target.value)}
            />
          </div>
        </>
      )}

      <div className="mori-quickstart-field">
        <label>
          {t("quickstart.api_key_label")}
          {envGroqDetected && provider === "groq" && (
            <span className="mori-quickstart-optional-tag">{t("quickstart.optional_when_env")}</span>
          )}
          <a href={info.helpUrl} target="_blank" rel="noopener noreferrer" className="mori-quickstart-help-link">
            {t("quickstart.where_get_key")} ↗
          </a>
        </label>
        <div className="mori-quickstart-key-input-row">
          <input
            type={showKey ? "text" : "password"}
            className="mori-input"
            placeholder={envGroqDetected && provider === "groq" ? t("quickstart.env_placeholder") : info.placeholder}
            value={keyText}
            onChange={(e) => {
              setKeyText(e.target.value);
              if (verify.kind !== "idle") setVerify({ kind: "idle" });
            }}
            autoFocus
          />
          <button
            type="button"
            className="mori-btn ghost small"
            onClick={() => setShowKey(!showKey)}
            title={showKey ? t("quickstart.hide_key") : t("quickstart.show_key")}
          >
            {showKey ? t("quickstart.hide_key") : t("quickstart.show_key")}
          </button>
        </div>
      </div>

      {provider === "openai_compat" && (
        <div className="mori-quickstart-field">
          <label>{t("quickstart.stt_key_label")}</label>
          <input
            type="password"
            className="mori-input"
            placeholder="gsk_... (留空 = 走本機 Whisper)"
            value={sttKeyText}
            onChange={(e) => setSttKeyText(e.target.value)}
          />
          <p className="mori-quickstart-field-hint">{t("quickstart.stt_key_hint")}</p>
        </div>
      )}

      <div className="mori-quickstart-verify-row">
        <button className="mori-btn" onClick={doVerify} disabled={!canVerify}>
          {verify.kind === "verifying" ? t("quickstart.verifying") : t("quickstart.verify_button")}
        </button>
        {verify.kind === "ok" && <span className="mori-quickstart-verify-msg ok">✓ {verify.msg}</span>}
        {verify.kind === "err" && <span className="mori-quickstart-verify-msg err">✗ {verify.msg}</span>}
      </div>

      <div className="mori-quickstart-footer">
        <button className="mori-btn" onClick={doSkip}>{t("quickstart.skip_button")}</button>
        <button className="mori-btn primary" onClick={doSave} disabled={!canSave}>{t("quickstart.save_button")}</button>
      </div>
    </>
  );
}

// ─── 儀式模式 ───────────────────────────────────────────────

interface RitualProps extends FormProps {
  step: number;
  setStep: (s: number) => void;
  onSwitchToDirect: () => void;
}

function RitualFlow(props: RitualProps) {
  const { t, step, setStep } = props;
  const total = 5;
  // 5 步:0=入林 1=點燈 2=獻鑰 3=驗印 4=甦醒
  const dots = (
    <div className="mori-quickstart-ritual-progress">
      {Array.from({ length: total }).map((_, i) => (
        <span
          key={i}
          className={`step-dot ${i === step ? "active" : ""} ${i < step ? "done" : ""}`}
        />
      ))}
    </div>
  );

  return (
    <div className="mori-quickstart-ritual">
      {step === 0 && <RitualStepEnter t={t} onNext={() => setStep(1)} onSkip={props.doSkip} onSwitchToDirect={props.onSwitchToDirect} dots={dots} />}
      {step === 1 && (
        <RitualStepLantern
          t={t}
          provider={props.provider} setProvider={props.setProvider}
          onBack={() => setStep(0)} onNext={() => setStep(2)}
          dots={dots}
        />
      )}
      {step === 2 && (
        <RitualStepOfferKey
          t={t}
          info={props.info}
          keyText={props.keyText} setKeyText={props.setKeyText}
          showKey={props.showKey} setShowKey={props.setShowKey}
          setVerify={props.setVerify}
          onBack={() => setStep(1)} onNext={() => setStep(3)}
          dots={dots}
        />
      )}
      {step === 3 && (
        <RitualStepVerify
          t={t}
          verify={props.verify}
          canVerify={props.canVerify}
          canSave={props.canSave}
          doVerify={props.doVerify}
          doSave={props.doSave}
          onBack={() => setStep(2)}
          dots={dots}
        />
      )}
      {step === 4 && <RitualStepAwaken t={t} />}
    </div>
  );
}

function RitualStepEnter({ t, onNext, onSkip, onSwitchToDirect, dots }: {
  t: any; onNext: () => void; onSkip: () => void; onSwitchToDirect: () => void; dots?: React.ReactNode;
}) {
  return (
    <div className="mori-quickstart-ritual-step">
      <p className="mori-ritual-narrative">{t("quickstart.ritual_enter_1")}</p>
      <p className="mori-ritual-narrative">{t("quickstart.ritual_enter_2")}</p>
      <p className="mori-ritual-narrative whisper">{t("quickstart.ritual_enter_3")}</p>
      {dots}
      <div className="mori-quickstart-footer">
        <button className="mori-btn ghost" onClick={onSkip}>{t("quickstart.ritual_dismiss")}</button>
        <button className="mori-btn ghost" onClick={onSwitchToDirect}>{t("quickstart.ritual_switch_direct")}</button>
        <button className="mori-btn primary" onClick={onNext}>{t("quickstart.ritual_enter_button")}</button>
      </div>
    </div>
  );
}

function RitualStepLantern({ t, provider, setProvider, onBack, onNext, dots }: {
  t: any; provider: Provider; setProvider: (p: Provider) => void; onBack: () => void; onNext: () => void; dots?: React.ReactNode;
}) {
  return (
    <div className="mori-quickstart-ritual-step">
      <p className="mori-ritual-narrative">{t("quickstart.ritual_lantern_1")}</p>
      <p className="mori-ritual-narrative whisper">{t("quickstart.ritual_lantern_2")}</p>
      <div className="mori-quickstart-provider-grid">
        {(Object.keys(PROVIDER_INFO) as Provider[]).map((p) => (
          <button
            key={p}
            className={`mori-quickstart-provider-card ${provider === p ? "active" : ""}`}
            onClick={() => setProvider(p)}
          >
            <span className="provider-name">{PROVIDER_INFO[p].label}</span>
            <span className="provider-hint">
              {p === "groq" ? t("quickstart.groq_hint") : t("quickstart.gemini_hint")}
            </span>
          </button>
        ))}
      </div>
      {dots}
      <div className="mori-quickstart-footer">
        <button className="mori-btn" onClick={onBack}>{t("quickstart.ritual_back")}</button>
        <button className="mori-btn primary" onClick={onNext}>{t("quickstart.ritual_lantern_button")}</button>
      </div>
    </div>
  );
}

function RitualStepOfferKey({
  t, info, keyText, setKeyText, showKey, setShowKey, setVerify, onBack, onNext, dots,
}: {
  t: any;
  info: typeof PROVIDER_INFO[Provider];
  keyText: string; setKeyText: (k: string) => void;
  showKey: boolean; setShowKey: (b: boolean) => void;
  setVerify: (v: VerifyState) => void;
  onBack: () => void; onNext: () => void;
  dots?: React.ReactNode;
}) {
  return (
    <div className="mori-quickstart-ritual-step">
      <p className="mori-ritual-narrative">{t("quickstart.ritual_offer_1")}</p>
      <p className="mori-ritual-narrative whisper">
        {t("quickstart.ritual_offer_2")}{" "}
        <a href={info.helpUrl} target="_blank" rel="noopener noreferrer">
          {t("quickstart.where_get_key")} ↗
        </a>
      </p>
      <div className="mori-quickstart-key-input-row">
        <input
          type={showKey ? "text" : "password"}
          className="mori-input"
          placeholder={info.placeholder}
          value={keyText}
          onChange={(e) => {
            setKeyText(e.target.value);
            setVerify({ kind: "idle" });
          }}
          autoFocus
        />
        <button
          type="button"
          className="mori-btn ghost small"
          onClick={() => setShowKey(!showKey)}
        >
          {showKey ? t("quickstart.hide_key") : t("quickstart.show_key")}
        </button>
      </div>
      {dots}
      <div className="mori-quickstart-footer">
        <button className="mori-btn" onClick={onBack}>{t("quickstart.ritual_back")}</button>
        <button
          className="mori-btn primary"
          onClick={onNext}
          disabled={keyText.trim().length <= 5}
        >{t("quickstart.ritual_offer_button")}</button>
      </div>
    </div>
  );
}

function RitualStepVerify({
  t, verify, canVerify, canSave, doVerify, doSave, onBack, dots,
}: {
  t: any;
  verify: VerifyState;
  canVerify: boolean;
  canSave: boolean;
  doVerify: () => void;
  doSave: () => void;
  onBack: () => void;
  dots?: React.ReactNode;
}) {
  // 自動觸發一次 verify(進入這步就驗)
  useEffect(() => {
    if (verify.kind === "idle" && canVerify) doVerify();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  return (
    <div className="mori-quickstart-ritual-step">
      <p className="mori-ritual-narrative">{t("quickstart.ritual_verify_1")}</p>
      <div className="mori-quickstart-verify-row centered">
        {verify.kind === "verifying" && (
          <span className="mori-ritual-pulse">✦ {t("quickstart.verifying")} ✦</span>
        )}
        {verify.kind === "ok" && (
          <span className="mori-quickstart-verify-msg ok">✓ {t("quickstart.ritual_verify_ok")}</span>
        )}
        {verify.kind === "err" && (
          <>
            <span className="mori-quickstart-verify-msg err">✗ {verify.msg}</span>
            <button className="mori-btn" onClick={doVerify}>{t("quickstart.verify_button")}</button>
          </>
        )}
      </div>
      {dots}
      <div className="mori-quickstart-footer">
        <button className="mori-btn" onClick={onBack}>{t("quickstart.ritual_back")}</button>
        <button className="mori-btn primary" onClick={doSave} disabled={!canSave}>
          {t("quickstart.ritual_verify_button")}
        </button>
      </div>
    </div>
  );
}

function RitualStepAwaken({ t }: { t: any }) {
  return (
    <div className="mori-quickstart-ritual-step awaken">
      <p className="mori-ritual-narrative whisper">{t("quickstart.ritual_awaken_1")}</p>
      <p className="mori-ritual-narrative">{t("quickstart.ritual_awaken_2")}</p>
      <div className="mori-ritual-sparkle">✦</div>
    </div>
  );
}

// ─── First-run detection ─────────────────────────────────────

/**
 * 偵測「需要顯示 Quickstart」 — 純看 flag:
 *   config.json `quickstart_completed === true` → 不顯示
 *   否則 → 顯示(第一次跑就會跳)
 *
 * 不偵測 API key 是因為:user 可能會故意清掉 key 重新設定,
 * 或想再走一次儀式流程 — 那是 Help 按鈕的工作,不是 auto 偵測。
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
  // 清掉舊版的 quickstart_skipped flag(已不用)
  delete cfg.quickstart_skipped;
  await invoke("config_write", { text: JSON.stringify(cfg, null, 2) });
}
