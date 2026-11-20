use crate::geometry::perimeter;
use crate::text::title_case;

pub fn describe(sides: &[u32]) -> String {
    format!("{} {}", title_case("total"), perimeter(sides))
}
