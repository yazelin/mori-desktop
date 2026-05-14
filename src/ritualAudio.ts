/// <reference types="vite/client" />
//
// 儀式氛圍音 — 強制 window-level 單例。
//
// 之前 HMR 留下舊 singleton + 新 singleton 同時跑導致疊播 + toggle 失效。
// 修法:audio element 掛 window.__moriRitualAudio,每次新 module load 進來
// 先把全域舊 element 強殺,確保整個 webview process 內**只一個**在播。
//
// 行為:
// - 進儀式 → 隨機抽 3 條音檔之一,loop 播
// - 切離 / 關 toggle / HMR reload → stop 並丟掉 element
// - 沒合成 fallback、沒 race condition

const TRACKS = [
  "/audio/leberch-film-517381.mp3",
  "/audio/leberch-ambient-517427.mp3",
  "/audio/leberch-soft-soft-music-522730.mp3",
];
const VOLUME = 0.4;

declare global {
  interface Window {
    __moriRitualAudio?: HTMLAudioElement | null;
    __moriRitualMuted?: boolean;
  }
}

// Module load:先強殺舊 element(來自 HMR 前一個 module instance)
if (typeof window !== "undefined" && window.__moriRitualAudio) {
  try {
    window.__moriRitualAudio.pause();
    window.__moriRitualAudio.src = "";
  } catch {}
  window.__moriRitualAudio = null;
}

class RitualAudio {
  // audio graph 給 visualizer:source → gain(volume) → analyser(freq data)→ destination
  // gainNode 控音量,analyser 給 freq bins,藏在 ritualAudio 內部
  private audioCtx: AudioContext | null = null;
  private source: MediaElementAudioSourceNode | null = null;
  private gainNode: GainNode | null = null;
  private analyser: AnalyserNode | null = null;

  /** Promise resolve = 真的播了;reject = browser autoplay 擋了,需 user gesture */
  startAmbient(): Promise<void> {
    if (window.__moriRitualAudio) return Promise.resolve();
    const url = TRACKS[Math.floor(Math.random() * TRACKS.length)];
    const el = new Audio(url);
    el.loop = true;
    el.crossOrigin = "anonymous";
    window.__moriRitualAudio = el;

    // graph setup 在 play 前,避免播放中途 reroute 害音檔重播
    try {
      this.audioCtx = new AudioContext();
      this.source = this.audioCtx.createMediaElementSource(el);
      this.gainNode = this.audioCtx.createGain();
      this.gainNode.gain.value = window.__moriRitualMuted ? 0 : VOLUME;
      this.analyser = this.audioCtx.createAnalyser();
      this.analyser.fftSize = 256;
      this.analyser.smoothingTimeConstant = 0.5;
      this.source.connect(this.gainNode);
      this.gainNode.connect(this.analyser);
      this.analyser.connect(this.audioCtx.destination);
    } catch (e) {
      console.warn("[ritualAudio] graph setup failed:", e);
    }

    // AudioContext autoplay policy:剛建出可能是 suspended(沒 user gesture),
    // 顯式 resume(若 user gesture 在 React mount 前已發生,這會成功)
    return Promise.resolve()
      .then(() => this.audioCtx?.resume())
      .then(() => el.play())
      .catch((e) => {
        // play 失敗最常是 autoplay policy。清掉 audio 讓下次 user-gesture call 重新試
        console.info("[ritualAudio] play blocked (likely autoplay policy):", e);
        this.stopAmbient();
        throw e;
      });
  }

  stopAmbient(): void {
    const el = window.__moriRitualAudio;
    if (el) {
      try { el.pause(); el.src = ""; } catch {}
    }
    window.__moriRitualAudio = null;
    // 拆 audio graph
    try { this.source?.disconnect(); } catch {}
    try { this.gainNode?.disconnect(); } catch {}
    try { this.analyser?.disconnect(); } catch {}
    try { this.audioCtx?.close(); } catch {}
    this.source = null;
    this.gainNode = null;
    this.analyser = null;
    this.audioCtx = null;
  }

  setMuted(muted: boolean): void {
    window.__moriRitualMuted = muted;
    // 走 gainNode(在 audio graph 內)— audio el 的 volume 在 setupAudioGraph
    // 後不再控訊號(訊號走 graph),所以一定要走 gainNode
    if (this.gainNode && this.audioCtx) {
      const now = this.audioCtx.currentTime;
      this.gainNode.gain.cancelScheduledValues(now);
      this.gainNode.gain.linearRampToValueAtTime(muted ? 0 : VOLUME, now + 0.1);
    }
    // graph 還沒建好就直接動 element volume 保底
    const el = window.__moriRitualAudio;
    if (el && !this.gainNode) el.volume = muted ? 0 : VOLUME;
  }

  isMuted(): boolean {
    return !!window.__moriRitualMuted;
  }

  /** Visualizer 用 — 取目前 32 bin freq data,長度跟 analyser.frequencyBinCount 一致。沒在播 = null */
  getAnalyser(): AnalyserNode | null {
    return this.analyser;
  }
}

export const ritualAudio = new RitualAudio();

// Vite HMR:dispose 階段強制停 — 不靠 module-load cleanup,而是 module
// unload 立刻清掉,避免 short window 兩個都在跑
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    try { ritualAudio.stopAmbient(); } catch {}
  });
}
