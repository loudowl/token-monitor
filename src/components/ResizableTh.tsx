import clsx from "clsx";
import type { ReactNode } from "react";

export default function ResizableTh({
  children,
  className,
  onResizeStart,
}: {
  children: ReactNode;
  className?: string;
  onResizeStart: (clientX: number) => void;
}) {
  return (
    <th className={clsx("px-3 py-2 font-medium relative select-none", className)}>
      {children}
      <div
        role="separator"
        aria-orientation="vertical"
        title="Drag to resize column"
        className="absolute top-0 right-0 h-full w-1.5 cursor-col-resize hover:bg-indigo-500/40 active:bg-indigo-500/60"
        onMouseDown={(e) => {
          e.preventDefault();
          e.stopPropagation();
          onResizeStart(e.clientX);
        }}
      />
    </th>
  );
}
