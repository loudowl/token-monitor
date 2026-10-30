import { useCallback, useEffect, useRef, useState } from "react";

/** Read-only, focusable prompt text — select all and copy from the Run column. */
export default function PromptCell({ text }: { text: string }) {
  const ref = useRef<HTMLTextAreaElement>(null);
  const [focused, setFocused] = useState(false);

  const syncHeight = useCallback(() => {
    const ta = ref.current;
    if (!ta || !focused) return;
    ta.style.height = "auto";
    ta.style.height = `${ta.scrollHeight}px`;
  }, [focused]);

  useEffect(() => {
    const ta = ref.current;
    if (!ta) return;
    if (focused) {
      syncHeight();
    } else {
      ta.style.height = "";
      ta.rows = 1;
    }
  }, [text, focused, syncHeight]);

  return (
    <textarea
      ref={ref}
      readOnly
      value={text}
      rows={1}
      spellCheck={false}
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
      onFocus={(e) => {
        setFocused(true);
        e.currentTarget.select();
      }}
      onBlur={() => setFocused(false)}
      className="w-full min-w-0 max-w-full bg-slate-950/40 border border-transparent hover:border-slate-700 focus:border-indigo-500/80 focus:bg-slate-950 rounded px-1.5 py-1 text-slate-200 resize-none cursor-text select-text text-xs leading-snug outline-none overflow-hidden focus:overflow-auto"
      title="Click to focus · select all to copy"
    />
  );
}
