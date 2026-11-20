from app.geometry import legacy_scale, perimeter
from app.strings import title_case
from tinyshape import Shape


def describe(sides: list[int]) -> str:
    grown = legacy_scale(sides, 2)
    square = Shape(4)
    return title_case("total") + " " + str(perimeter(grown)) + " " + str(square.corners)
