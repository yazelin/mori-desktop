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
  startAmbient(): void {
    // 已有 element 在 window registry 內 → 不重開
    if (window.__moriRitualAudio) return;
    const url = TRACKS[Math.floor(Math.random() * TRACKS.length)];
    const el = new Audio(url);
    el.loop = true;
    el.volume = window.__moriRitualMuted ? 0 : VOLUME;
    window.__moriRitualAudio = el;
    el.play().catch((e) => {
      console.warn("[ritualAudio] play failed:", e);
      if (window.__moriRitualAudio === el) window.__moriRitualAudio = null;
    });
  }

  stopAmbient(): void {
    const el = window.__moriRitualAudio;
    if (!el) return;
    try { el.pause(); el.src = ""; } catch {}
    window.__moriRitualAudio = null;
  }

  setMuted(muted: boolean): void {
    window.__moriRitualMuted = muted;
    const el = window.__moriRitualAudio;
    if (el) el.volume = muted ? 0 : VOLUME;
  }

  isMuted(): boolean {
    return !!window.__moriRitualMuted;
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
