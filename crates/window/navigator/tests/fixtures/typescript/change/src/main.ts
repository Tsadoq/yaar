import { perimeter } from "./geometry.js";
import { titleCase } from "./text/case.js";

export function describe(sides: number[]): string {
  return titleCase("total") + " " + perimeter(sides);
}
