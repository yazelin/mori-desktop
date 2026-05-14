// 儀式氛圍音 — 先試播 `/audio/ritual-ambient.mp3`(user 自己放音檔),
// 沒檔案就 fallback Web Audio API 合成的 ambient pad。
//
// 為什麼分兩段:
// - 真的音樂(user 從 Pixabay / Mixkit / FreeSound 下載放進 public/audio/)
//   音質、層次、情緒都遠勝合成
// - 沒音檔時 fallback synth,至少有東西(雖然 user 反映像白噪音)
//
// 簡單音檔 fallback 流程:
//   1. Vite 把 public/audio/ritual-ambient.mp3 serve 在 /audio/ 路徑
//   2. 進 ritual mode → HTMLAudioElement load + play
//   3. error / not-found → 退到 synth
//
// 找 CC0 / royalty-free 音樂:
//   - https://pixabay.com/music/search/ambient/ (CC0, 不需署名)
//   - https://mixkit.co/free-stock-music/mood/peaceful/
//   - https://freemusicarchive.org/genre/Ambient/

class RitualAudio {
  private ctx: AudioContext | null = null;
  private ambientNodes: AudioNode[] = [];
  private masterGain: GainNode | null = null;
  private muted: boolean = false;

  // 音檔模式
  private audioEl: HTMLAudioElement | null = null;
  private usingFile: boolean = false;

  private ensureContext(): AudioContext {
    if (!this.ctx) {
      const AC = window.AudioContext || (window as any).webkitAudioContext;
      this.ctx = new AC();
      this.masterGain = this.ctx.createGain();
      this.masterGain.gain.value = this.muted ? 0 : 0.35;
      this.masterGain.connect(this.ctx.destination);
    }
    if (this.ctx.state === "suspended") this.ctx.resume();
    return this.ctx;
  }

  /** 先試音檔,沒就 synth fallback。多條音檔時隨機選一條(每次開儀式氣氛換)。 */
  startAmbient(): void {
    if (this.usingFile || this.ambientNodes.length > 0) return;
    this.tryStartFile();
  }

  private pickTrackUrl(): string {
    // public/audio/ 內的 bundle 音檔 — 加新檔案進這個 list 就會被隨機抽到
    const tracks = [
      "/audio/ritual-ambient.mp3",
      "/audio/ritual-ambient-2.mp3",
      "/audio/ritual-ambient-3.mp3",
    ];
    return tracks[Math.floor(Math.random() * tracks.length)];
  }

  private tryStartFile(): void {
    const el = new Audio(this.pickTrackUrl());
    el.loop = true;
    el.volume = this.muted ? 0 : 0.4;
    el.preload = "auto";

    // 任一個 error event 都 fallback 到 synth
    const fallback = () => {
      if (this.usingFile) return; // 已經切過了
      console.info("[ritualAudio] no /audio/ritual-ambient.mp3, falling back to synth pad");
      this.audioEl = null;
      this.startSynth();
    };
    el.addEventListener("error", fallback, { once: true });
    el.addEventListener("stalled", fallback, { once: true });

    el.play()
      .then(() => {
        this.audioEl = el;
        this.usingFile = true;
        console.info("[ritualAudio] playing /audio/ritual-ambient.mp3");
      })
      .catch((e) => {
        console.info("[ritualAudio] audio file play failed, fallback to synth:", e);
        fallback();
      });
  }

  private startSynth(): void {
    const ctx = this.ensureContext();
    const out = this.masterGain!;

    const freqs = [196.0, 293.66, 220.0]; // G3 / D4 / A3
    const detunes = [0, 7, -5];

    for (let i = 0; i < freqs.length; i++) {
      const osc = ctx.createOscillator();
      osc.type = "sine";
      osc.frequency.value = freqs[i];
      osc.detune.value = detunes[i];

      const g = ctx.createGain();
      g.gain.value = 0.0;
      g.gain.linearRampToValueAtTime(0.25, ctx.currentTime + 1.5);

      const lfo = ctx.createOscillator();
      lfo.frequency.value = 0.15 + i * 0.03;
      const lfoGain = ctx.createGain();
      lfoGain.gain.value = 3.0;
      lfo.connect(lfoGain);
      lfoGain.connect(osc.detune);

      osc.connect(g);
      g.connect(out);

      osc.start();
      lfo.start();

      this.ambientNodes.push(osc, lfo, g, lfoGain);
    }
  }

  stopAmbient(): void {
    // 停音檔
    if (this.audioEl) {
      try {
        this.audioEl.pause();
        this.audioEl.src = "";
      } catch {}
      this.audioEl = null;
    }
    this.usingFile = false;

    // 停 synth
    if (!this.ctx || this.ambientNodes.length === 0) return;
    const now = this.ctx.currentTime;
    for (const n of this.ambientNodes) {
      if (n instanceof GainNode) {
        n.gain.cancelScheduledValues(now);
        n.gain.setValueAtTime(n.gain.value, now);
        n.gain.linearRampToValueAtTime(0, now + 1.0);
      }
    }
    const nodes = this.ambientNodes;
    this.ambientNodes = [];
    setTimeout(() => {
      for (const n of nodes) {
        try {
          if ("stop" in n && typeof (n as any).stop === "function") (n as any).stop();
        } catch {}
        try { n.disconnect(); } catch {}
      }
    }, 1200);
  }

  setMuted(muted: boolean): void {
    this.muted = muted;
    if (this.audioEl) {
      this.audioEl.volume = muted ? 0 : 0.4;
    }
    if (this.masterGain && this.ctx) {
      const now = this.ctx.currentTime;
      this.masterGain.gain.cancelScheduledValues(now);
      this.masterGain.gain.linearRampToValueAtTime(muted ? 0 : 0.35, now + 0.2);
    }
  }

  isMuted(): boolean {
    return this.muted;
  }
}

export const ritualAudio = new RitualAudio();
