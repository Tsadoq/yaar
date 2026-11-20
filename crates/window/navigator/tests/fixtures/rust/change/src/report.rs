use crate::geometry::perimeter;
use crate::text::title_case;

pub fn describe(sides: &[u32]) -> String {
    let total = perimeter(sides);
    let word = title_case("total");
    word + " " + &total.to_string()
}
