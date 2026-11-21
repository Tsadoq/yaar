def perimeter(sides: list[int]) -> int:
    return sum(sides)


def legacy_scale(sides: list[int], factor: int) -> list[int]:
    return [side * factor for side in sides]
