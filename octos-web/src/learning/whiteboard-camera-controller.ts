import type { WhiteboardRect } from "./whiteboard-placement";

export type WhiteboardCameraRequestSource =
  | "question-loading"
  | "course-end"
  | "course-restore"
  | "student-task";

export interface WhiteboardCameraRequest {
  key: string;
  source: WhiteboardCameraRequestSource;
  courseId: string;
  rect: WhiteboardRect;
}

export interface WhiteboardCameraDecision {
  action: "queued" | "replaced" | "ignored" | "applied";
  request: WhiteboardCameraRequest;
  reason?: "duplicate" | "course-settled" | "inactive-course" | "lower-priority";
}

type ScheduleFrame = (callback: () => void) => number;
type CancelFrame = (frame: number) => void;

const requestPriority: Record<WhiteboardCameraRequestSource, number> = {
  "question-loading": 4,
  "course-end": 3,
  "course-restore": 3,
  "student-task": 2,
};

function validRect(rect: WhiteboardRect): boolean {
  return [rect.x, rect.y, rect.width, rect.height].every(Number.isFinite)
    && rect.width > 0
    && rect.height > 0;
}

/**
 * The one owner of host-requested whiteboard camera moves.
 *
 * Lesson playback still supplies its normal teaching focus to the OLL view.
 * Everything added by the /learn host (questions, loading cards, course-end
 * framing, and student-task panels) must come through this controller so two
 * React effects cannot move the camera in an undefined order.
 */
export class WhiteboardCameraController {
  private pending: WhiteboardCameraRequest | null = null;
  private frame: number | null = null;
  private settledCourseId: string | null = null;
  private activeCourseId: string | null = null;
  private loadingCourseId: string | null = null;
  private readonly appliedKeys = new Set<string>();
  private readonly apply: (request: WhiteboardCameraRequest) => void;
  private readonly scheduleFrame: ScheduleFrame;
  private readonly cancelFrame: CancelFrame;
  private readonly report?: (decision: WhiteboardCameraDecision) => void;

  constructor(
    apply: (request: WhiteboardCameraRequest) => void,
    scheduleFrame: ScheduleFrame,
    cancelFrame: CancelFrame,
    report?: (decision: WhiteboardCameraDecision) => void,
  ) {
    this.apply = apply;
    this.scheduleFrame = scheduleFrame;
    this.cancelFrame = cancelFrame;
    this.report = report;
  }

  markCourseActive(courseId: string, force = false): boolean {
    if (
      !force
      && this.loadingCourseId !== null
      && this.loadingCourseId !== courseId
    ) return false;
    const enteredLoadingCourse = this.loadingCourseId === courseId;
    this.activeCourseId = courseId;
    if (enteredLoadingCourse || force) this.loadingCourseId = null;
    if (this.settledCourseId === courseId) this.settledCourseId = null;
    return enteredLoadingCourse;
  }

  canActivateCourse(courseId: string): boolean {
    return this.loadingCourseId === null || this.loadingCourseId === courseId;
  }

  request(request: WhiteboardCameraRequest): boolean {
    if (!validRect(request.rect) || this.appliedKeys.has(request.key)) {
      this.report?.({ action: "ignored", request, reason: "duplicate" });
      return false;
    }

    if (request.source === "question-loading") {
      this.activeCourseId = request.courseId;
      this.loadingCourseId = request.courseId;
    } else if (
      this.activeCourseId !== null
      && request.courseId !== this.activeCourseId
    ) {
      this.report?.({ action: "ignored", request, reason: "inactive-course" });
      return false;
    }

    if ((request.source === "student-task" || request.source === "course-restore")
      && this.settledCourseId === request.courseId) {
      this.report?.({ action: "ignored", request, reason: "course-settled" });
      return false;
    }

    if (request.source === "course-end" || request.source === "course-restore") {
      // Lock immediately, not after the animation frame. A task panel often
      // appears in the same React commit as course completion.
      this.settledCourseId = request.courseId;
    }

    const previous = this.pending;
    if (previous) {
      const previousPriority = requestPriority[previous.source];
      const nextPriority = requestPriority[request.source];
      if (nextPriority < previousPriority) {
        this.report?.({ action: "ignored", request, reason: "lower-priority" });
        return false;
      }
      this.pending = request;
      this.report?.({ action: "replaced", request });
      return true;
    }

    this.pending = request;
    this.report?.({ action: "queued", request });
    this.frame = this.scheduleFrame(() => {
      this.frame = null;
      const next = this.pending;
      this.pending = null;
      if (!next || this.appliedKeys.has(next.key)) return;
      this.appliedKeys.add(next.key);
      this.apply(next);
      this.report?.({ action: "applied", request: next });
    });
    return true;
  }

  destroy(): void {
    if (this.frame !== null) this.cancelFrame(this.frame);
    this.frame = null;
    this.pending = null;
  }
}
