import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { StudentInputDock } from "./student-input-dock";

describe("StudentInputDock board references", () => {
  it("shows attached board content and lets the learner remove it before sending", () => {
    const remove = vi.fn();
    render(
      <StudentInputDock
        voiceState="idle"
        cameraActive={false}
        onMic={vi.fn()}
        onToggleCamera={vi.fn()}
        onSendText={vi.fn()}
        onSendImage={vi.fn()}
        references={[{ id: "ref-1", label: "截面公式" }]}
        onRemoveReference={remove}
      />,
    );

    expect(screen.getByLabelText("已引用的白板内容").textContent)
      .toContain("已引用：截面公式");
    fireEvent.click(screen.getByRole("button", { name: "移除引用：截面公式" }));
    expect(remove).toHaveBeenCalledWith("ref-1");
  });
});
