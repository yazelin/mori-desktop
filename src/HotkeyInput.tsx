// 熱鍵欄:點一下後直接按想用的組合鍵就填入(Esc 取消,Backspace / Delete 清空回預設)。
import { useState } from "react";
import { chordFromEvent } from "./hotkeyChord";

interface Props {
  value: string;
  placeholder: string;
  onChange: (chord: string) => void;
}

export function HotkeyInput({ value, placeholder, onChange }: Props) {
  const [capturing, setCapturing] = useState(false);
  return (
    <input
      type="text"
      readOnly
      className="mori-input"
      value={capturing ? "" : value}
      placeholder={capturing ? "請按下新的組合鍵…(Esc 取消,Backspace 清除)" : placeholder}
      onFocus={() => setCapturing(true)}
      onBlur={() => setCapturing(false)}
      onKeyDown={(e) => {
        if (!capturing) return;
        e.preventDefault();
        e.stopPropagation();
        if (!e.ctrlKey && !e.altKey && !e.shiftKey && !e.metaKey) {
          if (e.code === "Escape") {
            e.currentTarget.blur();
            return;
          }
          if (e.code === "Backspace" || e.code === "Delete") {
            onChange("");
            e.currentTarget.blur();
            return;
          }
        }
        const chord = chordFromEvent(e);
        if (chord) {
          onChange(chord);
          e.currentTarget.blur();
        }
      }}
    />
  );
}
