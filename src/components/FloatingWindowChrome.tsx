import { getCurrentWindow } from "@tauri-apps/api/window";
import type { ReactNode } from "react";

type ResizeDir =
  | "North"
  | "South"
  | "East"
  | "West"
  | "NorthEast"
  | "NorthWest"
  | "SouthEast"
  | "SouthWest";

function startResize(e: React.MouseEvent, direction: ResizeDir) {
  e.stopPropagation();
  e.preventDefault();
  void getCurrentWindow().startResizeDragging(direction);
}

function ResizeEdge({
  direction,
  className,
}: {
  direction: ResizeDir;
  className: string;
}) {
  return (
    <div
      aria-hidden
      onMouseDown={(e) => startResize(e, direction)}
      className={`absolute z-50 touch-none ${className}`}
    />
  );
}

/**
 * Borderless utility window: no traffic lights, OS-style edge resize via Tauri.
 * Place drag handles on inner content with data-tauri-drag-region.
 */
export default function FloatingWindowChrome({
  children,
  padded = true,
}: {
  children: ReactNode;
  /** Widget-style inset; settings uses full bleed inside shadow. */
  padded?: boolean;
}) {
  return (
    <div
      className={
        padded
          ? "relative h-screen w-screen p-1 box-border overflow-hidden"
          : "relative h-screen w-screen box-border overflow-hidden"
      }
    >
      <ResizeEdge
        direction="North"
        className="top-0 left-0 right-0 h-2 cursor-n-resize"
      />
      <ResizeEdge
        direction="South"
        className="bottom-0 left-0 right-0 h-2 cursor-s-resize"
      />
      <ResizeEdge
        direction="West"
        className="left-0 top-2 bottom-2 w-2 cursor-w-resize"
      />
      <ResizeEdge
        direction="East"
        className="right-0 top-2 bottom-2 w-2 cursor-e-resize"
      />
      <ResizeEdge
        direction="NorthWest"
        className="top-0 left-0 w-3 h-3 cursor-nw-resize"
      />
      <ResizeEdge
        direction="NorthEast"
        className="top-0 right-0 w-3 h-3 cursor-ne-resize"
      />
      <ResizeEdge
        direction="SouthWest"
        className="bottom-0 left-0 w-3 h-3 cursor-sw-resize"
      />
      <ResizeEdge
        direction="SouthEast"
        className="bottom-0 right-0 w-3 h-3 cursor-se-resize"
      />
      <div
        className={
          padded
            ? "relative z-10 h-full w-full rounded-2xl border border-surface-border bg-surface shadow-2xl backdrop-blur-md overflow-hidden flex flex-col"
            : "relative z-10 h-full w-full rounded-xl border border-slate-800 bg-slate-950 shadow-2xl overflow-hidden flex flex-col"
        }
      >
        {children}
      </div>
    </div>
  );
}
