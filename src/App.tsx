import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Phase =
  | { kind: "idle" }
  | { kind: "recording"; started_at_ms: number }
  | { kind: "transcribing" }
  | { kind: "done"; transcript: string }
  | { kind: "error"; message: string };

function App() {
  const [coreVersion, setCoreVersion] = useState<string>("");
  const [phaseLabel, setPhaseLabel] = useState<string>("");
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [recElapsed, setRecElapsed] = useState<number>(0);
  const [audioLevel, setAudioLevel] = useState<number>(0);

  useEffect(() => {
    invoke<string>("mori_version").then(setCoreVersion).catch(() => setCoreVersion("(unavailable)"));
    invoke<string>("mori_phase").then(setPhaseLabel).catch(() => setPhaseLabel("(unavailable)"));
    invoke<boolean>("has_groq_key").then(setHasKey).catch(() => setHasKey(false));
    invoke<Phase>("current_phase").then(setPhase).catch(() => {});

    const unlistenPhase = listen<Phase>("phase-changed", (event) => {
      setPhase(event.payload);
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
        {phase.kind === "done" && (
          <>
            <div className="hero-dot done" />
            <p className="hero-text">完成</p>
            <p className="transcript">{phase.transcript || "(空白)"}</p>
            <p className="hero-hint">按熱鍵錄下一段</p>
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
          手動觸發(等同熱鍵)
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
