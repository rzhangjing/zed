import { describe, expect, it, vi } from "vitest";
import {
  WhiteboardCameraController,
  type WhiteboardCameraDecision,
  type WhiteboardCameraRequest,
} from "./whiteboard-camera-controller";

const rect = { x: 100, y: 120, width: 800, height: 600 };

function request(
  source: WhiteboardCameraRequest["source"],
  key: string,
  courseId = "course-2",
): WhiteboardCameraRequest {
  return { source, key, courseId, rect };
}

function controllerProbe() {
  const apply = vi.fn();
  const decisions: WhiteboardCameraDecision[] = [];
  let scheduled: (() => void) | null = null;
  const controller = new WhiteboardCameraController(
    apply,
    (callback) => {
      scheduled = callback;
      return 1;
    },
    vi.fn(),
    (decision) => decisions.push(decision),
  );
  return {
    apply,
    controller,
    decisions,
    flush: () => {
      const callback = scheduled as (() => void) | null;
      scheduled = null;
      callback?.();
    },
  };
}

describe("WhiteboardCameraController", () => {
  it("applies one course-end request and rejects the task that appears with it", () => {
    const probe = controllerProbe();

    expect(probe.controller.request(request("course-end", "end-2"))).toBe(true);
    expect(probe.controller.request(request("student-task", "task-2"))).toBe(false);
    probe.flush();

    expect(probe.apply).toHaveBeenCalledTimes(1);
    expect(probe.apply).toHaveBeenCalledWith(request("course-end", "end-2"));
    expect(probe.decisions).toContainEqual(expect.objectContaining({
      action: "ignored",
      reason: "course-settled",
      request: expect.objectContaining({ key: "task-2" }),
    }));
  });

  it("lets a new pending course replace an older course-end request", () => {
    const probe = controllerProbe();

    probe.controller.request(request("course-end", "end-1", "course-1"));
    probe.controller.request(request("question-loading", "loading-2", "course-2"));
    probe.flush();

    expect(probe.apply).toHaveBeenCalledTimes(1);
    expect(probe.apply).toHaveBeenCalledWith(
      request("question-loading", "loading-2", "course-2"),
    );
  });

  it("keeps an older course from reclaiming the camera after new-course loading was shown", () => {
    const probe = controllerProbe();

    probe.controller.request(request("question-loading", "loading-2", "course-2"));
    probe.flush();
    expect(
      probe.controller.request(request("course-end", "late-end-1", "course-1")),
    ).toBe(false);
    probe.flush();

    expect(probe.apply).toHaveBeenCalledTimes(1);
    expect(probe.apply).toHaveBeenCalledWith(
      request("question-loading", "loading-2", "course-2"),
    );
    expect(probe.decisions).toContainEqual(expect.objectContaining({
      action: "ignored",
      reason: "inactive-course",
      request: expect.objectContaining({ key: "late-end-1" }),
    }));
  });

  it("reports when the loaded course actually enters playback", () => {
    const probe = controllerProbe();

    probe.controller.request(request("question-loading", "loading-2", "course-2"));
    probe.flush();

    expect(probe.controller.canActivateCourse("course-1")).toBe(false);
    expect(probe.controller.canActivateCourse("course-2")).toBe(true);
    expect(probe.controller.markCourseActive("course-1")).toBe(false);
    expect(probe.controller.markCourseActive("course-2")).toBe(true);
    expect(probe.controller.markCourseActive("course-2")).toBe(false);
  });

  it("allows task framing again after replay makes the course active", () => {
    const probe = controllerProbe();

    probe.controller.request(request("course-restore", "restore-2"));
    probe.flush();
    probe.controller.markCourseActive("course-2");
    expect(probe.controller.request(request("student-task", "task-replay-2"))).toBe(true);
    probe.flush();

    expect(probe.apply).toHaveBeenCalledTimes(2);
  });

  it("does not reinterpret a course that just ended as a restored course", () => {
    const probe = controllerProbe();

    probe.controller.request(request("course-end", "end-2"));
    probe.flush();
    expect(probe.controller.request(request("course-restore", "restore-2"))).toBe(false);
    probe.flush();

    expect(probe.apply).toHaveBeenCalledTimes(1);
    expect(probe.decisions).toContainEqual(expect.objectContaining({
      action: "ignored",
      reason: "course-settled",
      request: expect.objectContaining({ key: "restore-2" }),
    }));
  });

  it("does not apply the same camera event twice", () => {
    const probe = controllerProbe();

    probe.controller.request(request("question-loading", "loading-2"));
    probe.flush();
    expect(probe.controller.request(request("question-loading", "loading-2"))).toBe(false);
    probe.flush();

    expect(probe.apply).toHaveBeenCalledTimes(1);
  });
});
