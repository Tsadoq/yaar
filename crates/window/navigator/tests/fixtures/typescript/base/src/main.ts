import { perimeter, legacyScale } from "./geometry.js";
import { titleCase } from "./strings.js";
import type { Shape } from "tinyshape";

export function describe(sides: number[]): string {
  const grown = legacyScale(sides, 2);
  const square: Shape = { corners: 4 };
  return titleCase("total") + " " + perimeter(grown) + " " + square.corners;
}
