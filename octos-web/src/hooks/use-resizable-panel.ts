import { useState, useCallback, useEffect, useRef } from "react";

interface ResizablePanelOptions {
  minWidth?: number;
  maxWidth?: number;
  defaultWidth?: number;
  storageKey?: string;
  side?: "left" | "right";
}

const KEYBOARD_STEP = 16;

/**
 * Resizable side-panel hook.
 *
 * - Pointer Events (pointerdown/move/up) so drag works with mouse, touch
 *   and pen.
 * - Keyboard support: spread `handleProps` onto the handle element to
 *   expose an accessible separator with arrow, Home, and End controls.
 */
export function useResizablePanel({
  minWidth = 280,
  maxWidth = 900,
  defaultWidth = 360,
  storageKey = "octos_panel_width",
  side = "right",
}: ResizablePanelOptions = {}) {
  const [width, setWidth] = useState(() => {
    if (storageKey) {
      const saved = localStorage.getItem(storageKey);
      if (saved) {
        const n = parseInt(saved, 10);
        if (n >= minWidth && n <= maxWidth) return n;
      }
    }
    return defaultWidth;
  });

  const [isMaximized, setIsMaximized] = useState(false);
  const isDragging = useRef(false);
  const startX = useRef(0);
  const startWidth = useRef(0);
  const activeDragCleanup = useRef<(() => void) | null>(null);

  useEffect(() => {
    if (storageKey && !isMaximized) {
      localStorage.setItem(storageKey, String(width));
    }
  }, [width, storageKey, isMaximized]);

  const onMouseDown = useCallback(
    (event: React.MouseEvent) => {
      if (event.button !== 0) return;
      event.preventDefault();
      activeDragCleanup.current?.();
      isDragging.current = true;
      startX.current = event.clientX;
      startWidth.current = width;
      const previousCursor = document.body.style.cursor;
      const previousUserSelect = document.body.style.userSelect;

      const onMouseMove = (moveEvent: MouseEvent) => {
        if (!isDragging.current) return;
        const delta =
          side === "right"
            ? startX.current - moveEvent.clientX
            : moveEvent.clientX - startX.current;
        setWidth(Math.min(maxWidth, Math.max(minWidth, startWidth.current + delta)));
      };

      let finished = false;
      const finishDragging = () => {
        if (finished) return;
        finished = true;
        isDragging.current = false;
        document.removeEventListener("mousemove", onMouseMove);
        document.removeEventListener("mouseup", finishDragging);
        window.removeEventListener("blur", finishDragging);
        document.body.style.cursor = previousCursor;
        document.body.style.userSelect = previousUserSelect;
        if (activeDragCleanup.current === finishDragging) {
          activeDragCleanup.current = null;
        }
      };

      document.addEventListener("mousemove", onMouseMove);
      document.addEventListener("mouseup", finishDragging);
      window.addEventListener("blur", finishDragging);
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
      activeDragCleanup.current = finishDragging;
    },
    [width, minWidth, maxWidth, side],
  );

  const onPointerDown = useCallback(
    (event: React.PointerEvent) => {
      // Some older PointerEvent shims omit isPrimary; only an explicit false
      // identifies an auxiliary pointer.
      if (event.isPrimary === false || event.button !== 0) return;
      event.preventDefault();
      activeDragCleanup.current?.();
      isDragging.current = true;
      startX.current = event.clientX;
      startWidth.current = width;
      const previousCursor = document.body.style.cursor;
      const previousUserSelect = document.body.style.userSelect;

      const onPointerMove = (moveEvent: PointerEvent) => {
        if (!isDragging.current) return;
        const delta =
          side === "right"
            ? startX.current - moveEvent.clientX
            : moveEvent.clientX - startX.current;
        setWidth(Math.min(maxWidth, Math.max(minWidth, startWidth.current + delta)));
      };

      let finished = false;
      const finishDragging = () => {
        if (finished) return;
        finished = true;
        isDragging.current = false;
        document.removeEventListener("pointermove", onPointerMove);
        document.removeEventListener("pointerup", finishDragging);
        document.removeEventListener("pointercancel", finishDragging);
        window.removeEventListener("blur", finishDragging);
        document.body.style.cursor = previousCursor;
        document.body.style.userSelect = previousUserSelect;
        if (activeDragCleanup.current === finishDragging) {
          activeDragCleanup.current = null;
        }
      };

      document.addEventListener("pointermove", onPointerMove);
      document.addEventListener("pointerup", finishDragging);
      document.addEventListener("pointercancel", finishDragging);
      window.addEventListener("blur", finishDragging);
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
      activeDragCleanup.current = finishDragging;
    },
    [width, minWidth, maxWidth, side],
  );

  useEffect(
    () => () => {
      activeDragCleanup.current?.();
    },
    [],
  );

  const onKeyDown = useCallback(
    (event: React.KeyboardEvent) => {
      let next: number | null = null;
      if (event.key === "Home") next = minWidth;
      else if (event.key === "End") next = maxWidth;
      else if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
        const physicalDelta = event.key === "ArrowRight" ? KEYBOARD_STEP : -KEYBOARD_STEP;
        const widthDelta = side === "left" ? physicalDelta : -physicalDelta;
        next = Math.min(maxWidth, Math.max(minWidth, width + widthDelta));
      }
      if (next === null) return;
      event.preventDefault();
      setWidth(next);
    },
    [maxWidth, minWidth, side, width],
  );

  const toggleMaximize = useCallback(() => {
    setIsMaximized((value) => !value);
  }, []);

  useEffect(() => {
    if (!isMaximized) return;
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") setIsMaximized(false);
    };
    document.addEventListener("keydown", handler);
    return () => document.removeEventListener("keydown", handler);
  }, [isMaximized]);

  const effectiveWidth = isMaximized ? "100%" : `${width}px`;

  const handleProps = {
    role: "separator",
    "aria-orientation": "vertical",
    "aria-valuenow": Math.round(width),
    "aria-valuemin": minWidth,
    "aria-valuemax": maxWidth,
    "aria-label": storageKey ? `Resize panel (${storageKey})` : "Resize panel",
    tabIndex: 0,
    onPointerDown,
    onKeyDown,
  } as const;

  return {
    width,
    effectiveWidth,
    isMaximized,
    onMouseDown,
    onPointerDown,
    onKeyDown,
    handleProps,
    toggleMaximize,
  };
}
