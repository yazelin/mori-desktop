import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type SkillCallSummary = {
  name: string;
  args_brief: string;
  success: boolean;
};

type Phase =
  | { kind: "idle" }
  | { kind: "recording"; started_at_ms: number }
  | { kind: "transcribing" }
  | { kind: "responding"; transcript: string }
  | {
      kind: "done";
      transcript: string;
      response: string;
      skill_calls: SkillCallSummary[];
    }
  | { kind: "error"; message: string };

function App() {
  const [coreVersion, setCoreVersion] = useState<string>("");
  const [phaseLabel, setPhaseLabel] = useState<string>("");
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [recElapsed, setRecElapsed] = useState<number>(0);
  const [audioLevel, setAudioLevel] = useState<number>(0);
  const [convLength, setConvLength] = useState<number>(0);

  const refreshConvLength = () => {
    invoke<number>("conversation_length")
      .then(setConvLength)
      .catch(() => setConvLength(0));
  };

  useEffect(() => {
    invoke<string>("mori_version").then(setCoreVersion).catch(() => setCoreVersion("(unavailable)"));
    invoke<string>("mori_phase").then(setPhaseLabel).catch(() => setPhaseLabel("(unavailable)"));
    invoke<boolean>("has_groq_key").then(setHasKey).catch(() => setHasKey(false));
    invoke<Phase>("current_phase").then(setPhase).catch(() => {});
    refreshConvLength();

    const unlistenPhase = listen<Phase>("phase-changed", (event) => {
      setPhase(event.payload);
      // 每次 phase 變化(尤其轉到 done 時)順便刷一下對話長度
      refreshConvLength();
    });
    const unlistenLevel = listen<number>("audio-level", (event) => {
      setAudioLevel(event.payload);
    });
    return () => {
      unlistenPhase.then((f) => f());
      unlistenLevel.then((f) => f());
    };
  }, []);

  useEffect(() => {
    if (phase.kind !== "recording") {
      setRecElapsed(0);
      return;
    }
    const tick = () => setRecElapsed(Math.floor((Date.now() - phase.started_at_ms) / 1000));
    tick();
    const id = setInterval(tick, 250);
    return () => clearInterval(id);
  }, [phase]);

  const onToggle = () => {
    invoke("toggle").catch((e) => console.error("toggle failed", e));
  };

  const onReset = () => {
    invoke("reset_conversation")
      .then(() => {
        setConvLength(0);
        setPhase({ kind: "idle" });
      })
      .catch((e) => console.error("reset failed", e));
  };

  return (
    <main className="container">
      <header>
        <h1>Mori</h1>
        <p className="subtitle">森林精靈 Mori 的桌面身體</p>
      </header>

      <section className={`hero hero-${phase.kind}`}>
        {phase.kind === "idle" && (
          <>
            <div className="hero-dot" />
            <p className="hero-text">待命中</p>
            <p className="hero-hint">按 <kbd>F8</kbd> 開始講話</p>
          </>
        )}
        {phase.kind === "recording" && (
          <>
            <div className="hero-dot pulse" />
            <p className="hero-text">錄音中… {recElapsed}s</p>
            <LevelMeter level={audioLevel} />
            <p className="hero-hint">再按一次熱鍵停止並送出</p>
          </>
        )}
        {phase.kind === "transcribing" && (
          <>
            <div className="hero-dot spin" />
            <p className="hero-text">轉錄中…</p>
            <p className="hero-hint">Whisper turbo 通常幾秒</p>
          </>
        )}
        {phase.kind === "responding" && (
          <>
            <div className="hero-dot spin" />
            <p className="hero-text">Mori 正在思考…</p>
            <div className="speech-block">
              <span className="speech-label">你說</span>
              <p className="speech-text">{phase.transcript || "(空白)"}</p>
            </div>
            <p className="hero-hint">gpt-oss-120b 通常 1-2 秒</p>
          </>
        )}
        {phase.kind === "done" && (
          <>
            <div className="hero-dot done" />
            <p className="hero-text">完成</p>
            <div className="speech-block">
              <span className="speech-label">你說</span>
              <p className="speech-text">{phase.transcript || "(空白)"}</p>
            </div>
            <div className="speech-block mori">
              <span className="speech-label">Mori</span>
              <p className="speech-text">{phase.response || "(無回應)"}</p>
              {phase.skill_calls && phase.skill_calls.length > 0 && (
                <div className="skill-badges">
                  {phase.skill_calls.map((sc, i) => (
                    <span
                      key={i}
                      className={`skill-badge ${sc.success ? "" : "failed"}`}
                      title={sc.args_brief}
                    >
                      {sc.success ? "🔧" : "⚠️"} {sc.name}
                      {sc.args_brief ? ` (${sc.args_brief})` : ""}
                    </span>
                  ))}
                </div>
              )}
            </div>
            <p className="hero-hint">按 F8 / 按鈕錄下一段</p>
          </>
        )}
        {phase.kind === "error" && (
          <>
            <div className="hero-dot error" />
            <p className="hero-text">出錯了</p>
            <p className="error-msg">{phase.message}</p>
            <p className="hero-hint">按熱鍵重試</p>
          </>
        )}
      </section>

      <section className="actions">
        <button onClick={onToggle} className="toggle-btn">
          手動觸發(等同 F8)
        </button>
        <button
          onClick={onReset}
          className="toggle-btn reset-btn"
          disabled={convLength === 0}
          title="清掉本次對話歷史(長期記憶不動)"
        >
          重新開始對話
        </button>
      </section>

      <section className="status">
        <div className="status-row">
          <span className="label">core</span>
          <span className="value">{coreVersion || "..."}</span>
        </div>
        <div className="status-row">
          <span className="label">phase</span>
          <span className="value">{phaseLabel || "..."}</span>
        </div>
        <div className="status-row">
          <span className="label">groq</span>
          <span className={`value ${hasKey ? "ok" : "warn"}`}>
            {hasKey === null ? "..." : hasKey ? "ready" : "no key"}
          </span>
        </div>
        <div className="status-row">
          <span className="label">history</span>
          <span className="value">{convLength} msgs</span>
        </div>
      </section>
    </main>
  );
}

// Real-time audio RMS bar. `level` is 0..=1 (linear amplitude).
// We map to dBFS for a more useful visual scale (whisper-friendly speech is
// usually -30..-10 dBFS, full silence is -∞).
function LevelMeter({ level }: { level: number }) {
  // Convert linear to a percent on a log-ish scale that puts whisper speech
  // around 50–80% of the bar.
  const db = level > 0 ? 20 * Math.log10(level) : -90;
  // Map -60dB → 0%, 0dB → 100%
  const pct = Math.max(0, Math.min(100, ((db + 60) / 60) * 100));
  // Color hint: green for normal speech, amber if near silence
  const tooQuiet = db < -45;
  return (
    <div className="level-meter">
      <div
        className={`level-fill ${tooQuiet ? "quiet" : ""}`}
        style={{ width: `${pct}%` }}
      />
      {tooQuiet && (
        <p className="level-hint">音量太小,Whisper 可能會幻想 “Thank you”</p>
      )}
    </div>
  );
}

export default App;
