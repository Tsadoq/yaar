pub fn perimeter(sides: &[u32]) -> u32 {
    sides.iter().sum()
}

pub fn legacy_scale(sides: &[u32], factor: u32) -> Vec<u32> {
    sides.iter().map(|side| side * factor).collect()
}
