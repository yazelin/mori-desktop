// 儀式氛圍音效 — Web Audio API 合成。
//
// 桌面版**比網頁版簡化**:只一條 ambient pad(3 個低頻 sine + LFO vibrato),
// 沒步驟 chime / 完成和弦,讓敘事為主、音場為輔。
// 只在 ritual mode 啟用,direct mode 不播。

class RitualAudio {
  private ctx: AudioContext | null = null;
  private ambientNodes: AudioNode[] = [];
  private masterGain: GainNode | null = null;
  private muted: boolean = false;

  /** lazy init — 第一次 play 時建 AudioContext(避免 autoplay 政策擋) */
  private ensureContext(): AudioContext {
    if (!this.ctx) {
      const AC = window.AudioContext || (window as any).webkitAudioContext;
      this.ctx = new AC();
      this.masterGain = this.ctx.createGain();
      // 音量拉高(0.18 → 0.35)— 原本太小,加上低頻被喇叭過濾,user 完全聽不到
      this.masterGain.gain.value = this.muted ? 0 : 0.35;
      this.masterGain.connect(this.ctx.destination);
    }
    // 如果被 browser 暫停了(idle 太久),resume
    if (this.ctx.state === "suspended") this.ctx.resume();
    return this.ctx;
  }

  /** 開始 ambient pad — 3 個 sine 重疊 + 緩 LFO,持續播放 */
  startAmbient(): void {
    if (this.ambientNodes.length > 0) return; // already playing
    const ctx = this.ensureContext();
    const out = this.masterGain!;

    // 3 個音疊在 audible range(原本 G1=49Hz 太低,筆電喇叭播不出)
    // 改成 G3 / D4 / A3 — 中低音 pad,溫的、不刺,user 聽得到
    const freqs = [196.0, 293.66, 220.0]; // G3 / D4 / A3
    const detunes = [0, 7, -5]; // cents

    for (let i = 0; i < freqs.length; i++) {
      const osc = ctx.createOscillator();
      osc.type = "sine";
      osc.frequency.value = freqs[i];
      osc.detune.value = detunes[i];

      const g = ctx.createGain();
      g.gain.value = 0.0;
      // 緩 fade-in 1.5s — 各 voice gain 0.25(原 0.2 太輕)
      g.gain.linearRampToValueAtTime(0.25, ctx.currentTime + 1.5);

      // LFO vibrato 0.15Hz 微調 detune,讓 pad 不死板
      const lfo = ctx.createOscillator();
      lfo.frequency.value = 0.15 + i * 0.03;
      const lfoGain = ctx.createGain();
      lfoGain.gain.value = 3.0; // ±3 cents
      lfo.connect(lfoGain);
      lfoGain.connect(osc.detune);

      osc.connect(g);
      g.connect(out);

      osc.start();
      lfo.start();

      this.ambientNodes.push(osc, lfo, g, lfoGain);
    }
  }

  /** 淡出 + 停 */
  stopAmbient(): void {
    if (!this.ctx || this.ambientNodes.length === 0) return;
    const now = this.ctx.currentTime;
    // 找出 gain node 做 fade-out
    for (const n of this.ambientNodes) {
      if (n instanceof GainNode) {
        n.gain.cancelScheduledValues(now);
        n.gain.setValueAtTime(n.gain.value, now);
        n.gain.linearRampToValueAtTime(0, now + 1.0);
      }
    }
    // 1.2s 後完全 disconnect
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
    if (this.masterGain) {
      const now = this.ctx!.currentTime;
      this.masterGain.gain.cancelScheduledValues(now);
      this.masterGain.gain.linearRampToValueAtTime(muted ? 0 : 0.35, now + 0.2);
    }
  }

  isMuted(): boolean {
    return this.muted;
  }
}

// 單例,跨 Quickstart 重 render 不重建
export const ritualAudio = new RitualAudio();
