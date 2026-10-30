import { useCallback, useEffect, useRef, useState } from "react";

export function useResizableColumns<K extends string>(
  storageKey: string,
  defaults: Record<K, number>,
  minWidth = 48,
) {
  const keys = Object.keys(defaults) as K[];

  const [widths, setWidths] = useState<Record<K, number>>(() => {
    try {
      const raw = localStorage.getItem(storageKey);
      if (raw) {
        const parsed = JSON.parse(raw) as Partial<Record<K, number>>;
        return { ...defaults, ...parsed };
      }
    } catch {
      /* ignore */
    }
    return { ...defaults };
  });

  useEffect(() => {
    try {
      localStorage.setItem(storageKey, JSON.stringify(widths));
    } catch {
      /* ignore */
    }
  }, [storageKey, widths]);

  const dragRef = useRef<{
    key: K;
    startX: number;
    startWidth: number;
  } | null>(null);

  const onResizeStart = useCallback(
    (key: K, clientX: number) => {
      dragRef.current = { key, startX: clientX, startWidth: widths[key] };
    },
    [widths],
  );

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      const d = dragRef.current;
      if (!d) return;
      const next = Math.max(minWidth, d.startWidth + (e.clientX - d.startX));
      setWidths((prev) => (prev[d.key] === next ? prev : { ...prev, [d.key]: next }));
    };
    const onUp = () => {
      dragRef.current = null;
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
    return () => {
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
  }, [minWidth]);

  return { widths, keys, onResizeStart };
}
