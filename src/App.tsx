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

type Mode = "active" | "background";

type BuildInfo = {
  sha: string;
  dirty: boolean;
  build_time: string;
  phase: string;
  version: string;
};

type WarmupState = "loading" | "ready" | "failed";
type SttInfo = {
  name: string;
  model: string;
  language: string | null;
};
type ChatProviderInfo = {
  name: string;
  model: string;
  warmup: WarmupState | null;
  stt: SttInfo;
};

function App() {
  const [coreVersion, setCoreVersion] = useState<string>("");
  const [phaseLabel, setPhaseLabel] = useState<string>("");
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [recElapsed, setRecElapsed] = useState<number>(0);
  const [audioLevel, setAudioLevel] = useState<number>(0);
  const [convLength, setConvLength] = useState<number>(0);
  const [mode, setMode] = useState<Mode>("active");
  const [buildInfo, setBuildInfo] = useState<BuildInfo | null>(null);
  const [chatProvider, setChatProvider] = useState<ChatProviderInfo | null>(null);
  const [warmup, setWarmup] = useState<WarmupState | null>(null);
  const [warmupStartedAt, setWarmupStartedAt] = useState<number | null>(null);
  const [warmupElapsed, setWarmupElapsed] = useState<number>(0);

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
    invoke<Mode>("current_mode").then(setMode).catch(() => {});
    invoke<BuildInfo>("build_info").then(setBuildInfo).catch(() => setBuildInfo(null));
    invoke<ChatProviderInfo>("chat_provider_info")
      .then((info) => {
        setChatProvider(info);
        // 後到 race:warm-up 可能在 React mount 前就完成,event 已經 emit 過。
        // 直接從 IPC 拿到的 snapshot 補一次,後續再靠 event 收 transition。
        if (info.warmup) {
          setWarmup(info.warmup);
          // mount 時就在 loading → 開始計時(approximate;不知道實際 start)
          if (info.warmup === "loading") setWarmupStartedAt(Date.now());
        }
      })
      .catch(() => setChatProvider(null));
    refreshConvLength();

    const unlistenPhase = listen<Phase>("phase-changed", (event) => {
      setPhase(event.payload);
      // 每次 phase 變化(尤其轉到 done 時)順便刷一下對話長度
      refreshConvLength();
      // Done / Error 之後 banner 就不該再顯示了
      if (event.payload.kind === "done" || event.payload.kind === "error") {
        setRetryStatus(null);
      }
    });
    const unlistenLevel = listen<number>("audio-level", (event) => {
      setAudioLevel(event.payload);
    });
    const unlistenRetry = listen<{
      attempt: number;
      max_attempts: number;
      wait_secs: number;
      reason: string;
      op: string;
    }>("rate-limit-wait", (event) => {
      setRetryStatus(event.payload);
      // wait_secs + 1 後自動消失(retry 開始就會被新事件覆蓋,或 phase 結束清掉)
      const ms = (event.payload.wait_secs + 1) * 1000;
      setTimeout(() => {
        setRetryStatus((curr) =>
          curr && curr.attempt === event.payload.attempt ? null : curr
        );
      }, ms);
    });
    // Phase 3A + 4C:Mori 在每輪開始抓 context(剪貼簿、反白文字)
    const unlistenContext = listen<{
      clipboard?: string | null;
      selected_text?: string | null;
    }>(
      "context-captured",
      (event) => {
        setLastContext(event.payload);
      },
    );
    // Phase 4B-2:Mode 切換(tray / IPC / set_mode skill)都會 emit 這個
    const unlistenMode = listen<Mode>("mode-changed", (event) => {
      setMode(event.payload);
    });
    // Phase 5A-1 hot-fix:啟動時若 default_provider=ollama,Tauri 會跑 warm-up
    // 並 emit 這個事件,UI 顯示「載入中 → 就緒/失敗」讓 user 看到 warm-up 真的有在動
    const unlistenWarmup = listen<WarmupState>("ollama-warmup", (event) => {
      setWarmup(event.payload);
      // 進 loading 時開始計時;結束時(ready/failed)凍結 elapsed 但保留顯示
      if (event.payload === "loading") {
        setWarmupStartedAt(Date.now());
      } else {
        setWarmupStartedAt(null);
      }
    });
    return () => {
      unlistenPhase.then((f) => f());
      unlistenLevel.then((f) => f());
      unlistenRetry.then((f) => f());
      unlistenContext.then((f) => f());
      unlistenMode.then((f) => f());
      unlistenWarmup.then((f) => f());
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

  // Warm-up elapsed timer — user 看得到 loading 已經幾秒,判斷是真在動還是 stuck
  useEffect(() => {
    if (warmupStartedAt === null) {
      setWarmupElapsed(0);
      return;
    }
    const tick = () => setWarmupElapsed(Math.floor((Date.now() - warmupStartedAt) / 1000));
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, [warmupStartedAt]);

  // Esc 取消錄音(只在主視窗 focused 時生效;global cancel 之後可走 portal)。
  useEffect(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && phase.kind === "recording") {
        e.preventDefault();
        invoke("cancel_recording").catch((err) =>
          console.error("cancel_recording failed", err),
        );
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [phase]);

  const [textInput, setTextInput] = useState<string>("");
  const [textOpen, setTextOpen] = useState<boolean>(false);
  const [retryStatus, setRetryStatus] = useState<{
    attempt: number;
    max_attempts: number;
    wait_secs: number;
    reason: string;
    op: string;
  } | null>(null);
  // Phase 3A + 4C:當 Mori 抓到剪貼簿 / 反白文字 時顯示
  const [lastContext, setLastContext] = useState<{
    clipboard?: string | null;
    selected_text?: string | null;
  } | null>(null);

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

  const onSubmitText = () => {
    const trimmed = textInput.trim();
    if (!trimmed) return;
    invoke("submit_text", { text: trimmed })
      .then(() => {
        setTextInput("");
        setTextOpen(false);
      })
      .catch((e) => console.error("submit_text failed", e));
  };

  const onToggleMode = () => {
    const next: Mode = mode === "active" ? "background" : "active";
    invoke("set_mode_cmd", { mode: next }).catch((e) =>
      console.error("set_mode_cmd failed", e),
    );
  };

  // mid-pipeline busy(Mori 在處理,使用者不能切斷)
  const pipelineBusy =
    phase.kind === "transcribing" || phase.kind === "responding";
  // 文字輸入相關按鈕 — 錄音中也鎖住(沒意義同時用兩個輸入)
  const textBusy = pipelineBusy || phase.kind === "recording";

  return (
    <main className="container">
      <header>
        <h1>Mori</h1>
        <p className="subtitle">森林精靈 Mori 的桌面身體</p>
      </header>

      {retryStatus && (
        <div className="retry-banner">
          {retryStatus.reason === "rate_limit" ? "Groq 限流" : "伺服器忙"} —
          等 {retryStatus.wait_secs}s 自動重試(第 {retryStatus.attempt}/
          {retryStatus.max_attempts} 次,{retryStatus.op})
        </div>
      )}

      <section className={`hero hero-${phase.kind}`}>
        {phase.kind === "idle" && (
          <>
            <div className="hero-dot" />
            <p className="hero-text">
              {mode === "background" ? "休眠中(麥克風已關)" : "待命中"}
            </p>
            <p className="hero-hint">
              {mode === "background" ? (
                <>按 <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Space</kbd> 叫醒並開始講話</>
              ) : (
                <>按 <kbd>Ctrl</kbd>+<kbd>Alt</kbd>+<kbd>Space</kbd> 開始講話</>
              )}
            </p>
          </>
        )}
        {phase.kind === "recording" && (
          <>
            <div className="hero-dot pulse" />
            <p className="hero-text">錄音中… {recElapsed}s</p>
            <LevelMeter level={audioLevel} />
            <p className="hero-hint">
              再按一次熱鍵 = 送出 / <kbd>Esc</kbd> = 取消(不送)
            </p>
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
            <p className="hero-hint">按 Ctrl+Alt+Space / 按鈕錄下一段</p>
          </>
        )}
        {phase.kind === "error" && (
          <>
            <div className="hero-dot error" />
            <p className="hero-text">出錯了</p>
            <p className="error-msg">{phase.message}</p>
            <p className="hero-hint">按 Ctrl+Alt+Space 重試</p>
          </>
        )}
      </section>

      <section className="actions">
        <button
          onClick={onToggle}
          className="toggle-btn"
          disabled={pipelineBusy || mode === "background"}
          title={
            mode === "background"
              ? "Mori 在休眠(麥克風關)— 按右側「醒醒」或熱鍵叫他"
              : undefined
          }
        >
          {phase.kind === "recording"
            ? "停止錄音"
            : mode === "background"
            ? "麥克風已關"
            : "手動觸發(等同 Ctrl+Alt+Space)"}
        </button>
        <button
          onClick={() => setTextOpen((v) => !v)}
          className="toggle-btn"
          disabled={textBusy}
          title="貼長文 / 打字輸入(語音不適合的場景);休眠時也能用"
        >
          {textOpen ? "收起文字輸入" : "貼文字"}
        </button>
        <button
          onClick={onToggleMode}
          className="toggle-btn"
          title={
            mode === "active"
              ? "讓 Mori 休眠 — 麥克風完全關閉,背景排程仍跑"
              : "叫醒 Mori — 重新允許麥克風"
          }
        >
          {mode === "active" ? "休眠(關麥克風)" : "醒醒(開麥克風)"}
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

      {textOpen && (
        <section className="text-input">
          <textarea
            value={textInput}
            onChange={(e) => setTextInput(e.target.value)}
            placeholder="貼文章 / 打需求,送出後跟語音輸入走同樣 pipeline。例如:&#10;「幫我摘要這篇:[長文...]」&#10;「翻成英文:[一段話]」"
            rows={6}
            disabled={textBusy}
            onKeyDown={(e) => {
              // Ctrl/Cmd + Enter 送出
              if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
                e.preventDefault();
                onSubmitText();
              }
            }}
          />
          <div className="text-input-actions">
            <span className="text-input-hint">
              {textInput.length} 字 · <kbd>Ctrl</kbd>+<kbd>Enter</kbd> 送出
            </span>
            <button
              onClick={onSubmitText}
              className="toggle-btn"
              disabled={textBusy || !textInput.trim()}
            >
              送出
            </button>
          </div>
        </section>
      )}

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
          <span className="label">build</span>
          <span
            className="value"
            title={
              buildInfo
                ? `built ${buildInfo.build_time}${buildInfo.dirty ? " · 含未提交變更" : ""}`
                : undefined
            }
          >
            {buildInfo
              ? `${buildInfo.sha}${buildInfo.dirty ? "*" : ""} · ${buildInfo.build_time}`
              : "..."}
          </span>
        </div>
        <div className="status-row">
          <span className="label">mode</span>
          <span className={`value ${mode === "background" ? "warn" : "ok"}`}>
            {mode === "background" ? "💤 休眠" : "🟢 清醒"}
          </span>
        </div>
        <div className="status-row">
          <span className="label">chat</span>
          <span
            className={`value ${
              chatProvider?.name === "groq"
                ? hasKey
                  ? "ok"
                  : "warn"
                : warmup === "ready"
                ? "ok"
                : warmup === "failed"
                ? "warn"
                : ""
            }`}
            title={
              chatProvider?.name === "groq"
                ? hasKey
                  ? "Groq API key 已就緒"
                  : "沒設 GROQ_API_KEY"
                : warmup === "ready"
                ? "Ollama 模型已載入,首次 chat 應該秒回"
                : warmup === "loading"
                ? "Ollama 正在把模型載進 RAM,完成後才不會 cold-start timeout"
                : warmup === "failed"
                ? "Ollama warm-up 失敗 — daemon 沒跑?model 沒下載?"
                : undefined
            }
          >
            {chatProvider
              ? `${chatProvider.name} · ${chatProvider.model}${
                  chatProvider.name === "ollama"
                    ? warmup === "loading"
                      ? ` · 🔄 載入中 ${warmupElapsed}s`
                      : warmup === "ready"
                      ? " · ✅ 就緒"
                      : warmup === "failed"
                      ? " · ⚠️ 失敗"
                      : ""
                    : hasKey === null
                    ? ""
                    : hasKey
                    ? " · ready"
                    : " · no key"
                }`
              : "..."}
          </span>
        </div>
        <div className="status-row">
          <span className="label">stt</span>
          <span
            className={`value ${
              chatProvider?.stt?.name === "whisper-local" ? "ok" : ""
            }`}
            title={
              chatProvider?.stt?.name === "whisper-local"
                ? `本機 whisper.cpp${
                    chatProvider.stt.language ? ` · 語言 ${chatProvider.stt.language}` : ""
                  } · ${chatProvider.stt.model}`
                : `Groq Whisper API · ${chatProvider?.stt?.model ?? ""}`
            }
          >
            {chatProvider?.stt
              ? chatProvider.stt.name === "whisper-local"
                ? `🏠 ${shortBasename(chatProvider.stt.model)}`
                : `☁ ${chatProvider.stt.model}`
              : "..."}
          </span>
        </div>
        <div className="status-row">
          <span className="label">history</span>
          <span className="value">{convLength} msgs</span>
        </div>
        <div className="status-row">
          <span className="label">clipboard</span>
          <span
            className={`value ${
              lastContext?.clipboard ? "ok" : ""
            }`}
            title={lastContext?.clipboard ?? "上一輪沒抓到剪貼簿內容"}
          >
            {lastContext?.clipboard
              ? `📋 ${lastContext.clipboard.length} 字`
              : "—"}
          </span>
        </div>
        <div className="status-row">
          <span className="label">selection</span>
          <span
            className={`value ${
              lastContext?.selected_text ? "ok" : ""
            }`}
            title={lastContext?.selected_text ?? "上一輪沒抓到反白文字"}
          >
            {lastContext?.selected_text
              ? `🖱 ${lastContext.selected_text.length} 字`
              : "—"}
          </span>
        </div>
      </section>
    </main>
  );
}

// 把長路徑(如 /home/.../models/ggml-small.bin)縮成 ggml-small.bin 顯示用
function shortBasename(path: string): string {
  const last = path.split("/").pop() ?? path;
  return last;
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
