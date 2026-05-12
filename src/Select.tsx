// brand-3 follow-up: custom dropdown 取代 native <select>。
//
// 原因:Linux webkit2gtk 把 <select> 的 dropdown panel 用 GTK widget 渲染,
// 配色被 system GTK theme 鎖死 — 不論 CSS color-scheme / option { background }
// 怎麼設,light theme 下 dropdown 仍 dark(因為 system 是 dark GTK theme)。
// 這個 component 用 div 自繪 panel,全部走 CSS variable,跟 theme 切換。
//
// API 跟 native select 接近:value / options / onChange + className 保留 mori-input 樣式。

import { useEffect, useRef, useState } from "react";
import { IconClose } from "./icons";

export type SelectOption = { value: string; label: string };

type Props = {
  value: string;
  options: SelectOption[];
  onChange: (v: string) => void;
  className?: string;
  placeholder?: string;
  disabled?: boolean;
  allowEmpty?: boolean; // 若 true,加一個「(無)」option 對應 value=""
  emptyLabel?: string;
};

export function Select({
  value,
  options,
  onChange,
  className,
  placeholder,
  disabled,
  allowEmpty,
  emptyLabel = "(無)",
}: Props) {
  const [open, setOpen] = useState(false);
  const [hoverIdx, setHoverIdx] = useState(-1);
  const wrapRef = useRef<HTMLDivElement>(null);

  const allOpts: SelectOption[] = allowEmpty
    ? [{ value: "", label: emptyLabel }, ...options]
    : options;
  const current = allOpts.find((o) => o.value === value);

  // 開啟時 hoverIdx 跳到目前值
  useEffect(() => {
    if (open) {
      const idx = allOpts.findIndex((o) => o.value === value);
      setHoverIdx(idx >= 0 ? idx : 0);
    }
  }, [open]);

  // 點外面關閉 + 鍵盤導航
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (wrapRef.current && !wrapRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        setOpen(false);
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setHoverIdx((i) => Math.min(allOpts.length - 1, (i < 0 ? -1 : i) + 1));
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setHoverIdx((i) => Math.max(0, (i < 0 ? 1 : i) - 1));
      }
      if (e.key === "Enter") {
        e.preventDefault();
        if (hoverIdx >= 0 && hoverIdx < allOpts.length) {
          onChange(allOpts[hoverIdx].value);
          setOpen(false);
        }
      }
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open, hoverIdx, allOpts]);

  return (
    <div
      ref={wrapRef}
      className={`mori-select ${className || ""} ${disabled ? "disabled" : ""} ${open ? "open" : ""}`}
    >
      <button
        type="button"
        className="mori-select-trigger"
        onClick={() => !disabled && setOpen(!open)}
        disabled={disabled}
      >
        <span className="mori-select-value">
          {current ? (
            current.label
          ) : (
            <span className="mori-select-placeholder">{placeholder || "選擇..."}</span>
          )}
        </span>
        <span className="mori-select-caret" aria-hidden>
          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round">
            <path d="M6 9 L12 15 L18 9" />
          </svg>
        </span>
      </button>
      {open && (
        <div className="mori-select-panel">
          {allOpts.length === 0 ? (
            <div className="mori-select-empty">(沒有選項)</div>
          ) : (
            allOpts.map((opt, i) => (
              <button
                key={opt.value || "__empty__"}
                type="button"
                className={`mori-select-option ${opt.value === value ? "selected" : ""} ${i === hoverIdx ? "hover" : ""}`}
                onClick={() => {
                  onChange(opt.value);
                  setOpen(false);
                }}
                onMouseEnter={() => setHoverIdx(i)}
              >
                {opt.label}
              </button>
            ))
          )}
        </div>
      )}
    </div>
  );
}

// 給 ProfileEditor 那種「拿掉某 key 用 X 按鈕」的場景:額外提供
// clearable wrapper(顯示 ✕ 在 trigger 內當 value 不為空時)
export function SelectClearable(props: Props & { onClear?: () => void }) {
  const { onClear, ...rest } = props;
  return (
    <div className="mori-select-clearable">
      <Select {...rest} />
      {rest.value && onClear && (
        <button
          type="button"
          className="mori-select-clear"
          onClick={onClear}
          title="清空"
        ><IconClose width={11} height={11} /></button>
      )}
    </div>
  );
}
