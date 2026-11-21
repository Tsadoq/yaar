export function perimeter(sides: number[]): number {
  return sides.reduce((sum, side) => sum + side, 0);
}
