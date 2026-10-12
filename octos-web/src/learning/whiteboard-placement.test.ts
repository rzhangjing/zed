import { describe, expect, it } from "vitest";
import {
  findNewTopicWhiteboardPosition,
  findOpenWhiteboardPosition,
  type WhiteboardRect,
} from "./whiteboard-placement";

function intersects(left: WhiteboardRect, right: WhiteboardRect): boolean {
  return left.x < right.x + right.width
    && left.x + left.width > right.x
    && left.y < right.y + right.height
    && left.y + left.height > right.y;
}

describe("whiteboard placement", () => {
  it("keeps a new question and its loading card away from existing handwriting", () => {
    const ink = { x: 80, y: 70, width: 260, height: 300 };
    const position = findOpenWhiteboardPosition({
      preferred: { x: 40, y: 190 },
      width: 654,
      height: 220,
      occupied: [ink],
    });
    expect(intersects({ ...position, width: 654, height: 220 }, ink)).toBe(false);
  });

  it("avoids lesson nodes, existing questions, auxiliary cards, and ink together", () => {
    const occupied = [
      { x: 0, y: 0, width: 360, height: 260 },
      { x: 390, y: 0, width: 270, height: 130 },
      { x: 390, y: 160, width: 330, height: 360 },
      { x: 60, y: 290, width: 250, height: 220 },
    ];
    const position = findOpenWhiteboardPosition({
      preferred: { x: 180, y: 170 },
      width: 654,
      height: 220,
      occupied,
    });
    const card = { ...position, width: 654, height: 220 };
    expect(occupied.every((rect) => !intersects(card, rect))).toBe(true);
  });

  it("keeps the preferred position when the board area is empty", () => {
    expect(findOpenWhiteboardPosition({
      preferred: { x: 100, y: 200 },
      width: 270,
      height: 130,
      occupied: [],
    })).toEqual({ x: 100, y: 200 });
  });

  it("starts a new lesson beyond the complete occupied whiteboard instead of filling a nearby hole", () => {
    const occupied = [
      { x: 100, y: 90, width: 460, height: 360 },
      { x: 620, y: 90, width: 270, height: 130 },
      { x: 300, y: 520, width: 900, height: 260 },
    ];
    const position = findNewTopicWhiteboardPosition({
      width: 654,
      height: 220,
      occupied,
    });
    expect(position.x).toBeGreaterThanOrEqual(1_200 + 180);
    expect(position.y).toBe(90);
  });
});
