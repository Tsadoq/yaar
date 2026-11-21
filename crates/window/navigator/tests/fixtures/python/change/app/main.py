from app.geometry import perimeter
from app.text.case import title_case


def describe(sides: list[int]) -> str:
    return title_case("total") + " " + str(perimeter(sides))
