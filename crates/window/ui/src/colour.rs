use gpui_kit::*;

pub fn hex(value: u32) -> Hsla {
    rgb(value).into()
}

pub fn hexa(value: u32) -> Hsla {
    rgba(value).into()
}

/// xterm's colours: a 6 by 6 by 6 cube from 16 (a channel is 0, or 55 plus 40 a step), then 24
/// greys from 232 (8 plus 10 a step). Under 16 this gives xterm's own sixteen; the terminal pane
/// takes those from the theme.
pub fn indexed(index: u8) -> Hsla {
    const BASE: [u32; 16] = [
        0x000000, 0xcd0000, 0x00cd00, 0xcdcd00, 0x0000ee, 0xcd00cd, 0x00cdcd, 0xe5e5e5, 0x7f7f7f,
        0xff0000, 0x00ff00, 0xffff00, 0x5c5cff, 0xff00ff, 0x00ffff, 0xffffff,
    ];
    let step = |n: u8| if n == 0 { 0 } else { 55 + 40 * u32::from(n) };
    match index {
        0..=15 => hex(BASE[usize::from(index)]),
        16..=231 => {
            let n = index - 16;
            hex(step(n / 36) << 16 | step(n / 6 % 6) << 8 | step(n % 6))
        }
        232..=255 => {
            let grey = 8 + 10 * u32::from(index - 232);
            hex(grey << 16 | grey << 8 | grey)
        }
    }
}

pub fn rgb24(red: u8, green: u8, blue: u8) -> Hsla {
    hex(u32::from(red) << 16 | u32::from(green) << 8 | u32::from(blue))
}

fn luminance(colour: Hsla) -> f32 {
    let rgba = colour.to_rgb();
    let linear = |c: f32| {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(rgba.r) + 0.7152 * linear(rgba.g) + 0.0722 * linear(rgba.b)
}

/// What `colour` looks like laid over an opaque `ground`.
pub fn composited(colour: Hsla, ground: Hsla) -> Hsla {
    let (over, under) = (colour.to_rgb(), ground.to_rgb());
    let mix = |a: f32, b: f32| a * over.a + b * (1.0 - over.a);
    Rgba {
        r: mix(over.r, under.r),
        g: mix(over.g, under.g),
        b: mix(over.b, under.b),
        a: 1.0,
    }
    .into()
}

/// WCAG's contrast ratio of `text` on `ground`, each laid over `backdrop` first.
pub fn contrast(text: Hsla, ground: Hsla, backdrop: Hsla) -> f32 {
    let (a, b) = (
        luminance(composited(text, backdrop)),
        luminance(composited(ground, backdrop)),
    );
    let (lighter, darker) = if a > b { (a, b) } else { (b, a) };
    (lighter + 0.05) / (darker + 0.05)
}

#[cfg(test)]
mod tests {
    use super::{contrast, hex, hexa, indexed, rgb24};

    #[test]
    fn the_contrast_a_test_reads_is_wcags_and_sees_what_is_not_readable() {
        let (black, white) = (hex(0x000000), hex(0xffffff));
        assert!((contrast(black, white, white) - 21.0).abs() < 0.01);
        assert!((contrast(white, black, white) - 21.0).abs() < 0.01);
        assert!((contrast(white, white, black) - 1.0).abs() < 0.001);
        // WCAG's own tables give #777777 on white 4.48 to 1.
        assert!((contrast(hex(0x777777), white, white) - 4.48).abs() < 0.01);
        // Half black over white is mid grey: white text on it reads 4.0 to 1.
        let half = hexa(0x00000080);
        assert!((contrast(white, half, white) - 4.0).abs() < 0.02);
        assert!(contrast(hex(0xfafafa), hex(0xeab308), white) < 4.5);
    }

    #[test]
    fn the_numbered_colours_above_the_sixteen_are_xterms_and_a_value_is_itself() {
        assert_eq!(indexed(16), hex(0x000000));
        assert_eq!(indexed(21), hex(0x0000ff));
        assert_eq!(indexed(196), hex(0xff0000));
        assert_eq!(indexed(231), hex(0xffffff));
        assert_eq!(indexed(17), hex(0x00005f));
        assert_eq!(indexed(16 + 36), hex(0x5f0000));
        assert_eq!(indexed(16 + 6 * 2 + 3), hex(0x0087af));
        assert_eq!(indexed(232), hex(0x080808));
        assert_eq!(indexed(244), hex(0x808080));
        assert_eq!(indexed(255), hex(0xeeeeee));
        assert_eq!(indexed(1), hex(0xcd0000));
        assert_eq!(indexed(15), hex(0xffffff));
        assert_eq!(rgb24(0x12, 0x34, 0x56), hex(0x123456));
    }
}
