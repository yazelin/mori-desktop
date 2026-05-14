// 儀式氛圍音 — 極簡版。
//
// 只播一條音檔 `/audio/ritual-ambient.mp3`,loop。
// 沒有合成 fallback,沒有隨機選 track,沒有 fade-out。
// 故意這樣做:多東西會疊播 / 失控,user 已經被搞煩。
//
// 想換音檔:把新檔覆蓋 public/audio/ritual-ambient.mp3。

class RitualAudio {
  private audioEl: HTMLAudioElement | null = null;
  private muted: boolean = false;

  startAmbient(): void {
    // 已經在播 → skip
    if (this.audioEl) return;
    const el = new Audio("/audio/ritual-ambient.mp3");
    el.loop = true;
    el.volume = this.muted ? 0 : 0.4;
    this.audioEl = el;
    el.play().catch((e) => {
      console.warn("[ritualAudio] play failed:", e);
      this.audioEl = null;
    });
  }

  stopAmbient(): void {
    if (!this.audioEl) return;
    try {
      this.audioEl.pause();
      this.audioEl.src = "";
    } catch {}
    this.audioEl = null;
  }

  setMuted(muted: boolean): void {
    this.muted = muted;
    if (this.audioEl) {
      this.audioEl.volume = muted ? 0 : 0.4;
    }
  }

  isMuted(): boolean {
    return this.muted;
  }
}

export const ritualAudio = new RitualAudio();

// Vite HMR:module 重 load 時把舊 singleton 完全停掉,避免疊播
if (import.meta.hot) {
  import.meta.hot.dispose(() => {
    try { ritualAudio.stopAmbient(); } catch {}
  });
}
