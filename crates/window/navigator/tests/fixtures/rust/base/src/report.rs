use crate::geometry::{legacy_scale, perimeter};
use crate::strings::title_case;

pub fn describe(sides: &[u32]) -> String {
    let grown = legacy_scale(sides, 2);
    let label = title_case("total");
    let total = perimeter(&grown);
    label + " " + &total.to_string()
}
