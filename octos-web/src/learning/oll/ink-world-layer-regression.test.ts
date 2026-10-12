import { describe, expect, it, vi } from "vitest";
import { InkRuntime } from "octos-lesson-language/ink-runtime";
import type { CameraState } from "octos-lesson-language/web-runtime";

type RuntimeHarness = {
  options: { viewport: HTMLElement };
  editor: {
    display: { setDevicePixelRatio: ReturnType<typeof vi.fn> };
    queueRerender: ReturnType<typeof vi.fn>;
  };
  renderedCamera: CameraState;
  appliedPixelRatio?: number;
  resetEditorViewport: ReturnType<typeof vi.fn>;
};

const updateWorldLayer = (
  InkRuntime.prototype as unknown as {
    updateWorldLayer: (this: RuntimeHarness, camera: CameraState) => void;
  }
).updateWorldLayer;

function createHarness(renderedCamera: CameraState): RuntimeHarness {
  const viewport = document.createElement("div");
  return {
    options: { viewport },
    editor: {
      display: { setDevicePixelRatio: vi.fn() },
      queueRerender: vi.fn(async () => undefined),
    },
    renderedCamera,
    resetEditorViewport: vi.fn(),
  };
}

describe("ink camera synchronization", () => {
  it("redraws a fixed drawing surface at the board camera while panning", () => {
    const runtime = createHarness({ panX: 80, panY: 60, scale: 1 });

    updateWorldLayer.call(runtime, { panX: -100, panY: -60, scale: 1 });

    expect(runtime.renderedCamera).toEqual({ panX: -100, panY: -60, scale: 1 });
    expect(runtime.resetEditorViewport).toHaveBeenCalledOnce();
    expect(runtime.editor.queueRerender).toHaveBeenCalledOnce();
  });

  it("redraws at the zoom camera and screen DPR", () => {
    const runtime = createHarness({ panX: 80, panY: 60, scale: 1 });

    updateWorldLayer.call(runtime, { panX: 20, panY: 10, scale: 1.25 });

    expect(runtime.renderedCamera).toEqual({ panX: 20, panY: 10, scale: 1.25 });
    expect(runtime.resetEditorViewport).toHaveBeenCalledOnce();
    expect(runtime.editor.display.setDevicePixelRatio).toHaveBeenCalledWith(window.devicePixelRatio);
    expect(runtime.editor.queueRerender).toHaveBeenCalledOnce();
  });

  it("tracks consecutive camera frames without moving the editor root", () => {
    const runtime = createHarness({ panX: 0, panY: 0, scale: 1 });

    updateWorldLayer.call(runtime, { panX: -40, panY: -20, scale: 1 });
    updateWorldLayer.call(runtime, { panX: -80, panY: -40, scale: 1 });

    expect(runtime.renderedCamera).toEqual({ panX: -80, panY: -40, scale: 1 });
    expect(runtime.resetEditorViewport).toHaveBeenCalledTimes(2);
    expect(runtime.editor.queueRerender).toHaveBeenCalledTimes(2);
  });
});
