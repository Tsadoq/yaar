export function perimeter(sides: number[]): number {
  return sides.reduce((sum, side) => sum + side, 0);
}

export function legacyScale(sides: number[], factor: number): number[] {
  return sides.map((side) => side * factor);
}
