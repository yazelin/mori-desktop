// 把瀏覽器的按鍵事件轉成 Mori / tauri-plugin-global-shortcut 吃的快捷鍵字串
// (例如 "Ctrl+Alt+Space")。設定頁的熱鍵欄「按下組合鍵就填入」用。

const NAMED: Record<string, string> = {
  Space: "Space",
  Enter: "Enter",
  Tab: "Tab",
  Backspace: "Backspace",
  Delete: "Delete",
  Insert: "Insert",
  Home: "Home",
  End: "End",
  PageUp: "PageUp",
  PageDown: "PageDown",
  ArrowUp: "Up",
  ArrowDown: "Down",
  ArrowLeft: "Left",
  ArrowRight: "Right",
  Escape: "Escape",
  Minus: "-",
  Equal: "=",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  Comma: ",",
  Period: ".",
  Slash: "/",
  Backquote: "`",
};

/** 非修飾鍵的名稱;修飾鍵本身或不支援的鍵回 null。 */
export function keyNameFromCode(code: string): string | null {
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  return NAMED[code] ?? null;
}

export interface ChordEvent {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

/**
 * 回傳快捷鍵字串;還沒按到一般鍵、或只有單獨一般鍵(沒有修飾鍵且不是 F1~F24)時回 null,
 * 避免使用者把 "A" 這種會吃掉一般打字的鍵設成全域熱鍵。
 */
export function chordFromEvent(e: ChordEvent): string | null {
  const key = keyNameFromCode(e.code);
  if (!key) return null;
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  const isFunctionKey = /^F\d+$/.test(key);
  if (mods.length === 0 && !isFunctionKey) return null;
  return [...mods, key].join("+");
}

/** 同一個組合不分修飾鍵順序、大小寫,用來找出重複的熱鍵。 */
export function normalizeChord(chord: string): string {
  const parts = chord
    .split("+")
    .map((p) => p.trim().toLowerCase())
    .filter(Boolean);
  const order = ["ctrl", "control", "alt", "shift", "super", "meta", "cmd", "command"];
  const mods = parts.filter((p) => order.includes(p)).map((p) => (p === "control" ? "ctrl" : p));
  const keys = parts.filter((p) => !order.includes(p));
  return [...mods.sort(), ...keys].join("+");
}
