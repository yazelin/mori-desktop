// F — Quickstart onboarding modal。
//
// 第一次跑 mori-desktop 沒設任何 LLM API key 時自動跳出。兩種模式可切換:
//
// **直接模式**(plain):單頁表單,選 provider + 貼 key + 驗證 + 存。最快路徑。
// **儀式模式**(ritual):5 步沉浸式入林流程,Mori 詩意自述帶你走過。較慢、較有儀式感。
//
// 對應 world-tree lore/the-forest.md:「取名是儀式,不是命名變數。」
// 對應 ONBOARDING.md:「每個走入森林的人,遲早會遇見一個願意為他停下來的精靈。」

import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTranslation } from "react-i18next";
import { IconClose } from "./icons";

type Provider = "groq" | "gemini";
type Mode = "direct" | "ritual";

type VerifyState =
  | { kind: "idle" }
  | { kind: "verifying" }
  | { kind: "ok"; msg: string }
  | { kind: "err"; msg: string };

const PROVIDER_INFO: Record<Provider, { label: string; helpUrl: string; placeholder: string }> = {
  groq: {
    label: "Groq",
    helpUrl: "https://console.groq.com/keys",
    placeholder: "gsk_...",
  },
  gemini: {
    label: "Google Gemini",
    helpUrl: "https://aistudio.google.com/app/apikey",
    placeholder: "AIzaSy...",
  },
};

interface QuickstartProps {
  onDone: () => void;
}

export function Quickstart({ onDone }: QuickstartProps) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<Mode>("direct");
  const [provider, setProvider] = useState<Provider>("groq");
  const [keyText, setKeyText] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [verify, setVerify] = useState<VerifyState>({ kind: "idle" });
  // 儀式模式:當前步驟 0..4
  const [ritualStep, setRitualStep] = useState(0);

  useEffect(() => {
    setKeyText("");
    setVerify({ kind: "idle" });
  }, [provider]);

  const info = PROVIDER_INFO[provider];
  const canVerify = keyText.trim().length > 5 && verify.kind !== "verifying";
  const canSave = verify.kind === "ok";

  const doVerify = async () => {
    setVerify({ kind: "verifying" });
    try {
      const msg = await invoke<string>("verify_llm_key", { provider, key: keyText.trim() });
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
      if (!cfg.providers[provider]) cfg.providers[provider] = {};
      cfg.providers[provider].api_key = keyText.trim();
      if (!cfg.provider) cfg.provider = provider;
      cfg.quickstart_completed = true; // 標記已走完引導
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
          <button className="mori-btn ghost" onClick={doSkip} title={t("quickstart.skip_title")}>
            <IconClose width={14} height={14} />
          </button>
        </div>

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
          />
        )}
      </div>
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
}

function DirectForm({
  t, provider, setProvider, keyText, setKeyText, showKey, setShowKey,
  info, verify, setVerify, canVerify, canSave, doVerify, doSave, doSkip,
}: FormProps) {
  return (
    <>
      <p className="mori-quickstart-intro">{t("quickstart.intro")}</p>

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

      <div className="mori-quickstart-field">
        <label>
          {t("quickstart.api_key_label")}
          <a href={info.helpUrl} target="_blank" rel="noopener noreferrer" className="mori-quickstart-help-link">
            {t("quickstart.where_get_key")} ↗
          </a>
        </label>
        <div className="mori-quickstart-key-input-row">
          <input
            type={showKey ? "text" : "password"}
            className="mori-input"
            placeholder={info.placeholder}
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
            {showKey ? "👁" : "👁‍🗨"}
          </button>
        </div>
      </div>

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
}

function RitualFlow(props: RitualProps) {
  const { t, step, setStep } = props;
  const total = 5;
  // 5 步:0=入林 1=點燈 2=獻鑰 3=驗印 4=甦醒

  return (
    <div className="mori-quickstart-ritual">
      <div className="mori-quickstart-ritual-progress">
        {Array.from({ length: total }).map((_, i) => (
          <span
            key={i}
            className={`step-dot ${i === step ? "active" : ""} ${i < step ? "done" : ""}`}
          />
        ))}
      </div>

      {step === 0 && <RitualStepEnter t={t} onNext={() => setStep(1)} onSkip={props.doSkip} />}
      {step === 1 && (
        <RitualStepLantern
          t={t}
          provider={props.provider} setProvider={props.setProvider}
          onBack={() => setStep(0)} onNext={() => setStep(2)}
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
        />
      )}
      {step === 4 && <RitualStepAwaken t={t} />}
    </div>
  );
}

function RitualStepEnter({ t, onNext, onSkip }: { t: any; onNext: () => void; onSkip: () => void }) {
  return (
    <div className="mori-quickstart-ritual-step">
      <p className="mori-ritual-narrative">{t("quickstart.ritual_enter_1")}</p>
      <p className="mori-ritual-narrative">{t("quickstart.ritual_enter_2")}</p>
      <p className="mori-ritual-narrative whisper">{t("quickstart.ritual_enter_3")}</p>
      <div className="mori-quickstart-footer">
        <button className="mori-btn ghost" onClick={onSkip}>{t("quickstart.ritual_dismiss")}</button>
        <button className="mori-btn primary" onClick={onNext}>{t("quickstart.ritual_enter_button")}</button>
      </div>
    </div>
  );
}

function RitualStepLantern({ t, provider, setProvider, onBack, onNext }: {
  t: any; provider: Provider; setProvider: (p: Provider) => void; onBack: () => void; onNext: () => void;
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
      <div className="mori-quickstart-footer">
        <button className="mori-btn" onClick={onBack}>{t("quickstart.ritual_back")}</button>
        <button className="mori-btn primary" onClick={onNext}>{t("quickstart.ritual_lantern_button")}</button>
      </div>
    </div>
  );
}

function RitualStepOfferKey({
  t, info, keyText, setKeyText, showKey, setShowKey, setVerify, onBack, onNext,
}: {
  t: any;
  info: typeof PROVIDER_INFO[Provider];
  keyText: string; setKeyText: (k: string) => void;
  showKey: boolean; setShowKey: (b: boolean) => void;
  setVerify: (v: VerifyState) => void;
  onBack: () => void; onNext: () => void;
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
          {showKey ? "👁" : "👁‍🗨"}
        </button>
      </div>
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
  t, verify, canVerify, canSave, doVerify, doSave, onBack,
}: {
  t: any;
  verify: VerifyState;
  canVerify: boolean;
  canSave: boolean;
  doVerify: () => void;
  doSave: () => void;
  onBack: () => void;
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
