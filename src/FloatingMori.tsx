import { CSSProperties, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

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

type Mode = "agent" | "voice_input" | "background";

type Visual =
  | "sleeping"
  | "idle"
  | "recording"
  | "thinking"
  | "done"
  | "error";

const TRANSIENT_DURATION_MS: Record<"done" | "error", number> = {
  done: 1500,
  error: 2000,
};

// 轉錄原文泡泡顯示時長
const TRANSCRIPT_LABEL_MS = 3000;
// Profile 名稱顯示時長
const PROFILE_LABEL_MS = 1500;

// 麥克風 RMS 值通常在 0.01–0.20，需要 sqrt + 放大讓效果更明顯
const amplify = (v: number) => Math.sqrt(Math.min(v * 4, 1.0));

// 波紋觸發門檻 — 低於這個視為靜音不發波
const RIPPLE_THRESHOLD = 0.04;
// 波紋發射間隔 — 最快多久一次（防止暴雷洗版）
const RIPPLE_MIN_INTERVAL_MS = 180;
// 單個波紋存活時間
const RIPPLE_LIFETIME_MS = 1200;

// 5P-3: sprite-frame inline style — 走 4×4 row-major 兩軸動畫。
// 設計重點:
// - x 軸 (mori-sprite-x) 跑 4 frame in one row,duration = 整 sheet / 4
// - y 軸 (mori-sprite-y) 跑 4 row,duration = 整 sheet 時長
// - 兩軸都 steps(4) jump-end,以 (0, 0) → (-400%, -400%) wrap 回 (0, 0) 完成 loop
// - 這版簡化 不分 loop / one-shot,全 infinite(commit 4 toggle 時可改)
// - grid "1x1" → 不跑 animation,純 static
function spriteStyle(
  visual: Visual,
  spriteUrl: string | undefined,
  manifest: CharacterManifest | null,
): CSSProperties {
  if (!spriteUrl) return {};
  const grid = manifest?.sprite_spec?.grid ?? "4x4";
  if (grid === "1x1") {
    return {
      backgroundImage: `url("${spriteUrl}")`,
      backgroundSize: "100% 100%",
      backgroundRepeat: "no-repeat",
    };
  }
  const duration = manifest?.loop_durations_ms?.[visual] ?? 1600;
  return {
    backgroundImage: `url("${spriteUrl}")`,
    backgroundSize: "400% 400%",
    backgroundRepeat: "no-repeat",
    animationName: "mori-sprite-x, mori-sprite-y",
    animationDuration: `${duration / 4}ms, ${duration}ms`,
    animationTimingFunction: "steps(4), steps(4)",
    animationIterationCount: "infinite, infinite",
  };
}

function visualFor(
  mode: Mode,
  phase: Phase,
  transient: Visual | null,
): Visual {
  if (mode === "background") return "sleeping";
  if (transient) return transient;
  switch (phase.kind) {
    case "idle":
      return "idle";
    case "recording":
      return "recording";
    case "transcribing":
    case "responding":
      return "thinking";
    case "done":
    case "error":
      return "idle";
  }
}

const VISUAL_LABEL: Record<Visual, string> = {
  sleeping: "休眠中",
  idle: "在這",
  recording: "聽中",
  thinking: "想中",
  done: "完成",
  error: "出錯",
};

// 5P-3: Sprite 從 character pack 來,manifest + 各 state PNG data URL 從 IPC 拉。
// 不再 hardcode public/floating/ path,讓 user 能換角色 pack。

type CharacterManifest = {
  schema_version: string;
  package_name: string;
  display_name: string;
  version?: string;
  states: string[];
  optional_states?: string[];
  loop_modes?: Record<string, string>;       // "loop" | "one-shot"
  loop_durations_ms?: Record<string, number>;
  sprite_spec: {
    format: string;
    grid: string;                             // "4x4" / "1x1"
    total_size: string;
    frame_size: string;
    frame_order: string;
    background: string;
  };
};

function FloatingMori() {
  const [mode, setMode] = useState<Mode>("agent");
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [transient, setTransient] = useState<Visual | null>(null);

  // 5F-3A: 音量驅動的 aura（0.0–1.0，後端 ~30Hz emit）
  const [volume, setVolume] = useState(0);

  // 5F-3A 波紋：音量超過門檻時 spawn 一個 ripple element，CSS animation
  // 自動 fade out。lastRippleAtRef 限流避免每 33ms 都發一個。
  const [ripples, setRipples] = useState<Array<{ id: number; intensity: number }>>([]);
  const lastRippleAtRef = useRef(0);

  // 暫時性 info（有 timeout 會消失）
  const [infoLabel, setInfoLabel] = useState<string | null>(null);
  const [infoKey, setInfoKey] = useState(0);
  const showInfo = (text: string) => {
    setInfoLabel(text);
    setInfoKey((k) => k + 1);
  };

  // 持久性狀態 label（錄音中、轉錄中、處理中）
  const [statusLabel, setStatusLabel] = useState<string | null>(null);

  // 當前 profile 常駐標籤（Alt+N 設定後一直記著，錄音中持續顯示）
  const [currentProfileLabel, setCurrentProfileLabel] = useState<string>("");

  // 5J: 完整 chat bubble 改用獨立 chat_bubble window 顯示
  // (sprite window 永遠 160×160 不動,bubble 走另一個 Tauri window)。
  // 這裡只保留「目前是否有 bubble」的旗標 + dwell timer 控制。
  const [hasChatBubble, setHasChatBubble] = useState(false);

  // 5P-3: Character pack — manifest + 各 state 的 sprite data URL
  const [manifest, setManifest] = useState<CharacterManifest | null>(null);
  const [sprites, setSprites] = useState<Partial<Record<Visual, string>>>({});

  useEffect(() => {
    const loadCharacterPack = async () => {
      try {
        const [stem, m] = await invoke<[string, CharacterManifest]>("character_get_active");
        setManifest(m);
        // foreach state 抓 data URL
        const allStates: Visual[] = ["idle", "sleeping", "recording", "thinking", "done", "error"];
        const entries = await Promise.all(
          allStates.map(async (state) => {
            try {
              const url = await invoke<string>("character_sprite_data_url", {
                stem,
                state,
              });
              return [state, url] as const;
            } catch (e) {
              console.warn("[FloatingMori] failed to load sprite", state, e);
              return [state, ""] as const;
            }
          }),
        );
        const map: Partial<Record<Visual, string>> = {};
        for (const [s, u] of entries) {
          if (u) map[s] = u;
        }
        setSprites(map);
      } catch (e) {
        console.error("[FloatingMori] character_get_active failed", e);
      }
    };
    loadCharacterPack();
    // 5P-6: ConfigTab character picker 切換 active 後 emit 這個
    const unlistenChar = listen("character-changed", () => loadCharacterPack());
    return () => {
      unlistenChar.then((f) => f());
    };
  }, []);

  // ── 初始化 & 事件訂閱 ─────────────────────────────────────────────

  useEffect(() => {
    invoke<Mode>("current_mode").then(setMode).catch(() => {});
    invoke<Phase>("current_phase").then(setPhase).catch(() => {});

    const unlistenMode = listen<Mode>("mode-changed", (e) => setMode(e.payload));
    const unlistenPhase = listen<Phase>("phase-changed", (e) => setPhase(e.payload));

    // 5F-3A: 音量事件（main.rs 在錄音中每 ~33ms emit 一次）
    const unlistenVolume = listen<number>("audio-level", (e) => {
      const v = e.payload;
      setVolume(v);

      // 音量超過門檻 + 距離上一個波紋 > 限流間隔 → 發新波紋
      const now = performance.now();
      if (v >= RIPPLE_THRESHOLD && now - lastRippleAtRef.current >= RIPPLE_MIN_INTERVAL_MS) {
        lastRippleAtRef.current = now;
        const id = now;
        const intensity = amplify(v);
        setRipples((rs) => [...rs, { id, intensity }]);
        // 動畫結束後自動移除
        setTimeout(() => {
          setRipples((rs) => rs.filter((r) => r.id !== id));
        }, RIPPLE_LIFETIME_MS);
      }
    });

    // profile 切換："朋友閒聊 · groq" 格式
    const unlistenProfile = listen<string>("voice-input-profile-switched", (e) => {
      setCurrentProfileLabel(e.payload); // 持久記住
      showInfo(e.payload);               // 短暫顯示
      const t = setTimeout(() => setInfoLabel(null), PROFILE_LABEL_MS);
      return () => clearTimeout(t);
    });

    // 轉錄中 / 處理中狀態（後端 emit，有狀態就持續顯示直到下一個狀態）
    const unlistenStatus = listen<string>("voice-input-status", (e) => {
      setStatusLabel(e.payload);
    });

    return () => {
      unlistenMode.then((f) => f());
      unlistenPhase.then((f) => f());
      unlistenVolume.then((f) => f());
      unlistenProfile.then((f) => f());
      unlistenStatus.then((f) => f());
    };
  }, []);

  // 結束狀態時清掉 statusLabel（"轉錄中" / "處理中" 不應該留在 done 之後）
  useEffect(() => {
    if (phase.kind === "done" || phase.kind === "error" || phase.kind === "idle") {
      setStatusLabel(null);
    }
  }, [phase.kind]);

  // ── transient done / error flash ──────────────────────────────────

  useEffect(() => {
    if (phase.kind === "done") {
      setTransient("done");
      const t = setTimeout(() => setTransient(null), TRANSIENT_DURATION_MS.done);
      return () => clearTimeout(t);
    }
    if (phase.kind === "error") {
      setTransient("error");
      const t = setTimeout(() => setTransient(null), TRANSIENT_DURATION_MS.error);
      return () => clearTimeout(t);
    }
    setTransient(null);
  }, [phase]);

  // ── 5J: 完成後浮動提示 ────────────────────────────────────────
  // - VoiceInput mode: 短轉錄(≤40 字)→ infoLabel(頂端 chip);長轉錄 → chat_bubble window
  // - Agent mode: Mori 完整回應一律走 chat_bubble window(獨立 window,不受 sprite 限制)

  // sprite 視窗在 tauri.conf.json 寫死 160×160 且 resizable:false。
  // 不問 outerSize() — GNOME mutter 對 transparent+decorationless 視窗的 outerSize
  // 在不同時刻可能加上不同 shadow margin,會讓 bubble 每次距離 sprite 越來越遠。
  const SPRITE_SIZE = 160;
  const BUBBLE_WIDTH = 360;
  const BUBBLE_GAP_PX = 8;

  // 顯示 chat bubble:從 sprite 視窗位置算出 bubble 絕對座標,emit 給 chat_bubble window
  const showChatBubble = async (text: string) => {
    try {
      const win = getCurrentWindow();
      const pos = await win.outerPosition();
      const scale = await win.scaleFactor();
      const sprite_x = pos.x / scale;
      const sprite_y = pos.y / scale;
      // bubble 中心對齊 sprite 中心
      const bubble_x = Math.max(0, sprite_x + SPRITE_SIZE / 2 - BUBBLE_WIDTH / 2);
      const bubble_y = sprite_y + SPRITE_SIZE + BUBBLE_GAP_PX;
      await emit("chat-bubble-show", { text, x: bubble_x, y: bubble_y });
      setHasChatBubble(true);
    } catch (e) {
      console.error("show chat_bubble failed", e);
    }
  };

  const hideChatBubble = async () => {
    try {
      await emit("chat-bubble-hide");
    } catch (e) { console.error("hide chat_bubble failed", e); }
    setHasChatBubble(false);
  };

  useEffect(() => {
    if (phase.kind !== "done") return;

    if (mode === "voice_input" && phase.transcript.trim()) {
      const text = phase.transcript.trim();
      // 短文字直接 chip 顯示就好,避免開額外視窗
      if (text.length <= 40) {
        showInfo(text);
        const t = setTimeout(() => setInfoLabel(null), TRANSCRIPT_LABEL_MS);
        return () => clearTimeout(t);
      }
      // 長轉錄走 chat_bubble window(完整顯示讓使用者驗證 STT)
      showChatBubble(text);
      const t = setTimeout(hideChatBubble, 6000);
      return () => clearTimeout(t);
    }

    if (mode === "agent" && phase.response.trim()) {
      const text = phase.response.trim();
      showChatBubble(text);
      // 訊息越長給越久時間讀 — 每 30 字 +1 秒,base 5 秒,最多 15 秒
      const dwell = Math.min(15000, 5000 + Math.floor(text.length / 30) * 1000);
      const t = setTimeout(hideChatBubble, dwell);
      return () => clearTimeout(t);
    }
  }, [phase, mode]);

  // 錄音開始時清掉舊的 info label + chat bubble window,避免上輪的內容殘留
  useEffect(() => {
    if (phase.kind === "recording") {
      setInfoLabel(null);
      if (hasChatBubble) hideChatBubble();
    }
  }, [phase.kind]);

  // ── Drag ──────────────────────────────────────────────────────────

  const dragRef = useRef<{ x: number; y: number; armed: boolean } | null>(null);
  const DRAG_THRESHOLD_PX = 4;

  const onMouseDown = (e: React.MouseEvent) => {
    if (e.buttons !== 1) return;
    dragRef.current = { x: e.clientX, y: e.clientY, armed: true };
  };

  const onMouseMove = (e: React.MouseEvent) => {
    const d = dragRef.current;
    if (!d || !d.armed) return;
    const dx = Math.abs(e.clientX - d.x);
    const dy = Math.abs(e.clientY - d.y);
    if (dx > DRAG_THRESHOLD_PX || dy > DRAG_THRESHOLD_PX) {
      d.armed = false;
      invoke("plugin:window|start_dragging", { label: "floating" }).catch(
        (err) => console.error("start_dragging failed", err),
      );
    }
  };

  const onMouseUp = async () => {
    dragRef.current = null;
    // 拖動結束,通知 chat_bubble window 跟著移動到新位置(用 hardcoded sprite 尺寸算)
    if (hasChatBubble) {
      try {
        const win = getCurrentWindow();
        const pos = await win.outerPosition();
        const scale = await win.scaleFactor();
        const sprite_x = pos.x / scale;
        const sprite_y = pos.y / scale;
        await emit("sprite-moved", {
          x: Math.max(0, sprite_x + SPRITE_SIZE / 2 - BUBBLE_WIDTH / 2),
          y: sprite_y + SPRITE_SIZE + BUBBLE_GAP_PX,
        });
      } catch (e) {
        console.error("sync chat_bubble position after drag failed", e);
      }
    }
  };

  const onDoubleClick = async () => {
    try {
      const { WebviewWindow } = await import("@tauri-apps/api/webviewWindow");
      const main = await WebviewWindow.getByLabel("main");
      if (!main) return;
      const visible = await main.isVisible();
      if (visible) { await main.hide(); }
      else { await main.show(); await main.setFocus(); }
    } catch (e) {
      console.error("toggle main from floating failed", e);
    }
  };

  const visual = visualFor(mode, phase, transient);

  // 基底環不再 scale（避免 box-shadow 外溢出視窗被切），只用 --vol 控制
  // ::before 的發光強度。實際的「音量波動」由獨立的 ripple elements 表現。
  const auraStyle: CSSProperties | undefined =
    visual === "recording"
      ? ({ "--vol": amplify(volume).toFixed(3) } as CSSProperties)
      : undefined;

  // 5K-1b: 標籤顯示優先序(簡化版)
  //   infoLabel (時效性訊息:profile 切換 1.5s / done 結果) 最優先
  //   → statusLabel (轉錄中 / 處理中)
  //   → recording 中常駐顯示 profile 名稱(讓使用者知道按下這次會用哪個處理)
  //   idle 時不再常駐顯示 — 切換時 1.5s 即消失,sprite 保持乾淨
  const labelToShow: string | null =
    infoLabel
    ?? statusLabel
    ?? (visual === "recording" && currentProfileLabel ? `● ${currentProfileLabel}` : null);

  return (
    <div
      className={`mori-stage mori-${visual}`}
      onMouseDown={onMouseDown}
      onMouseMove={onMouseMove}
      onMouseUp={onMouseUp}
      onDoubleClick={onDoubleClick}
      title={`Mori — ${VISUAL_LABEL[visual]}\n拖曳:移動 / 雙擊:切顯示主視窗`}
    >
      {/* 5J: sprite-area — 永遠固定在 widget 左上 160×160,讓 sprite 不會
          因為 window 變寬 / 變高而跑位置。bubble / chip 浮在這之外。 */}
      <div className="mori-sprite-area">
        {/* 背景光暈：錄音中由音量驅動；其他狀態 CSS animation */}
        <div className="mori-aura" style={auraStyle} />

        {/* 5F-3A: 音量波紋層 — 音量超過門檻時 spawn ripple，向外擴散後 fade。
            最大擴張到 145px（< 160px 視窗），不會被切。 */}
        {visual === "recording" &&
          ripples.map((r) => (
            <div
              key={r.id}
              className="mori-ripple"
              style={{ "--ripple-intensity": r.intensity.toFixed(3) } as CSSProperties}
            />
          ))}

        {/* 5P-3: 角色 sprite container 套既有 state-specific transform animation
            (mori-breathe / mori-doze / mori-listen-bob 等),子層 frame 跑 sheet loop。
            兩層分開避免 animation property 互相覆蓋。動畫 ON 預設(commit 4 接 toggle)。
            loop_durations_ms 從 manifest 拿,placeholder 階段 16 格全是同一張看似不閃。 */}
        <div
          className={`mori-sprite mori-sprite-${visual}`}
          title={VISUAL_LABEL[visual]}
        >
          <div
            className="mori-sprite-frame"
            style={spriteStyle(visual, sprites[visual], manifest)}
          />
        </div>

        {/* 5J: 頂端 chip — profile 切換 / 狀態 / 短文字,在 sprite 上方,
            chip 隨 sprite-area 移動,window resize 不會跑掉。 */}
        {labelToShow && (
          <div key={`${labelToShow}-${infoKey}`} className="mori-info-label">
            {labelToShow}
          </div>
        )}
      </div>

      {/* 5J: Mori 完整回應現在用獨立 chat_bubble window 顯示
          (Wayland 上單窗 setSize + transparent 太不穩),這裡不再渲染。 */}
    </div>
  );
}

export default FloatingMori;
