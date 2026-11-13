//! Draws the tray icon: a rounded square showing the current volume. A built-in pixel font
//! keeps the app free of font files and licences.
//!
//! Two styles: [`Style::Colour`] (Windows: blue when connected, grey when not) and
//! [`Style::Template`] (macOS menu bar: a black shape with the number cut out, which macOS tints
//! black or white to match the menu bar; dimmed when not connected).

pub const SIZE: u32 = 64;

const CONNECTED_BG: [u8; 3] = [24, 119, 242];
const DISCONNECTED_BG: [u8; 3] = [110, 110, 110];

/// 5x7 glyphs, one row per entry, bit 4 = leftmost pixel.
fn glyph(c: char) -> [u8; 7] {
    match c {
        '0' => [0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110],
        '1' => [0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110],
        '2' => [0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111],
        '3' => [0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110],
        '4' => [0b00010, 0b00110, 0b01010, 0b10010, 0b11111, 0b00010, 0b00010],
        '5' => [0b11111, 0b10000, 0b11110, 0b00001, 0b00001, 0b10001, 0b01110],
        '6' => [0b00110, 0b01000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110],
        '7' => [0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000],
        '8' => [0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110],
        '9' => [0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00010, 0b01100],
        _ => [0, 0, 0, 0b11111, 0, 0, 0], // '-'
    }
}

/// Frames in the connecting animation: no dots, then one, two and three.
pub const CONNECTING_FRAMES: u8 = 4;

/// The connecting icon: the same square with three dots that fill in one by one (`frame` 0..=3).
pub fn render_connecting(frame: u8, style: Style) -> Vec<u8> {
    let bg = match style {
        Style::Colour => DISCONNECTED_BG,
        Style::Template => [0, 0, 0],
    };
    let mut px = square(style, bg, 1.0);
    let filled = usize::from(frame.min(CONNECTING_FRAMES - 1));
    let (r, gap) = (5.5f32, 16.0f32);
    let centre = SIZE as f32 / 2.0;
    for (k, cx) in [centre - gap, centre, centre + gap].into_iter().enumerate() {
        let on = k < filled;
        for y in 0..SIZE {
            for x in 0..SIZE {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - centre).powi(2)).sqrt();
                let cover = (r + 0.5 - d).clamp(0.0, 1.0);
                if cover == 0.0 {
                    continue;
                }
                let i = ((y * SIZE + x) * 4) as usize;
                match style {
                    // template: a filled dot is a hole; a waiting one is a faint hole
                    Style::Template => {
                        let keep = if on { 1.0 - cover } else { 1.0 - 0.45 * cover };
                        px[i + 3] = (f32::from(px[i + 3]) * keep).round() as u8;
                    }
                    // colour: white dots on grey; waiting ones only a little lighter than the square
                    Style::Colour => {
                        let amount = cover * if on { 1.0 } else { 0.3 };
                        for c in 0..3 {
                            px[i + c] = (f32::from(px[i + c]) * (1.0 - amount) + 255.0 * amount).round() as u8;
                        }
                    }
                }
            }
        }
    }
    px
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Colour,
    /// macOS "template image": only the alpha channel counts.
    Template,
}

/// How strongly a not-connected template icon is drawn (macOS shows it lighter).
const TEMPLATE_DIMMED: f32 = 0.45;

/// The style the tray uses on this platform.
pub const PLATFORM_STYLE: Style = if cfg!(target_os = "macos") { Style::Template } else { Style::Colour };

/// The rounded square on its own, with an anti-aliased edge (a little smaller in the menu bar,
/// like other menu bar icons).
fn square(style: Style, bg: [u8; 3], strength: f32) -> Vec<u8> {
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];
    let (half, radius) = if style == Style::Template { (27.0f32, 11.0f32) } else { (30.0f32, 14.0f32) };
    let centre = SIZE as f32 / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (dx, dy) =
                ((x as f32 + 0.5 - centre).abs() - (half - radius), (y as f32 + 0.5 - centre).abs() - (half - radius));
            let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt();
            let dist = outside + dx.max(dy).min(0.0) - radius;
            let alpha = (0.5 - dist).clamp(0.0, 1.0) * strength;
            let i = ((y * SIZE + x) * 4) as usize;
            px[i..i + 3].copy_from_slice(&bg);
            px[i + 3] = (alpha * 255.0).round() as u8;
        }
    }

    px
}

/// RGBA pixels (`SIZE` x `SIZE`) in the colour style. `None` or `connected == false` draws a grey
/// icon with a dash.
#[cfg(test)]
fn render(volume: Option<u8>, connected: bool) -> Vec<u8> {
    render_styled(volume, connected, Style::Colour)
}

pub fn render_styled(volume: Option<u8>, connected: bool, style: Style) -> Vec<u8> {
    let (bg, strength) = match style {
        Style::Colour => (if connected { CONNECTED_BG } else { DISCONNECTED_BG }, 1.0),
        Style::Template => ([0, 0, 0], if connected { 1.0 } else { TEMPLATE_DIMMED }),
    };
    let mut px = square(style, bg, strength);

    let text = match volume {
        Some(v) if connected => v.to_string(),
        _ => "-".to_string(),
    };
    let n = text.chars().count() as u32;
    let scale = match (style, n) {
        (Style::Colour, 1) => 6,
        (Style::Template, 1) => 5,
        _ => 4,
    };
    // colour: white digits on the square; template: digits cut out of it
    let ink: [u8; 4] = if style == Style::Template { [0, 0, 0, 0] } else { [255, 255, 255, 255] };
    let width = n * 5 * scale + (n - 1) * scale;
    let (x0, y0) = ((SIZE - width) / 2, (SIZE - 7 * scale) / 2);
    for (k, c) in text.chars().enumerate() {
        let gx = x0 + k as u32 * 6 * scale;
        for (row, bits) in glyph(c).iter().enumerate() {
            for col in 0..5u32 {
                if bits >> (4 - col) & 1 == 1 {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let (x, y) = (gx + col * scale + sx, y0 + row as u32 * scale + sy);
                            let i = ((y * SIZE + x) * 4) as usize;
                            px[i..i + 4].copy_from_slice(&ink);
                        }
                    }
                }
            }
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    fn white(px: &[u8]) -> usize {
        px.chunks_exact(4).filter(|p| p == &[255, 255, 255, 255]).count()
    }

    #[test]
    fn has_the_right_size_and_transparent_corners() {
        let px = render(Some(7), true);
        assert_eq!(px.len(), (SIZE * SIZE * 4) as usize);
        assert_eq!(px[3], 0, "top-left corner is transparent");
        let inside = ((4 * SIZE + SIZE / 2) * 4 + 3) as usize; // just below the middle of the top edge
        assert_eq!(px[inside], 255, "inside the shape");
        let above = ((SIZE / 2) * 4 + 3) as usize; // top row, above the shape
        assert_eq!(px[above], 0, "outside the shape");
    }

    #[test]
    fn every_volume_draws_digits_inside_the_icon() {
        for v in 0..=99u8 {
            let px = render(Some(v), true);
            assert!(white(&px) > 40, "volume {v} drew nothing");
        }
    }

    #[test]
    fn template_is_black_with_the_number_cut_out() {
        let px = render_styled(Some(7), true, Style::Template);
        assert!(px.chunks_exact(4).all(|p| p[..3] == [0, 0, 0]), "only black: macOS uses just the alpha");
        let centre = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
        assert_eq!(px[centre + 3], 0, "the 7's stroke is a hole in the shape");
        let corner_inside = ((8 * SIZE + 12) * 4 + 3) as usize;
        assert_eq!(px[corner_inside], 255, "the shape itself is solid");
        assert_eq!(px[3], 0, "outside is transparent");
        for v in 0..=16u8 {
            let p = render_styled(Some(v), true, Style::Template);
            let holes = p.chunks_exact(4).enumerate().filter(|(i, q)| {
                let (x, y) = ((*i as u32) % SIZE, (*i as u32) / SIZE);
                q[3] == 0 && (16..48).contains(&x) && (16..48).contains(&y)
            });
            assert!(holes.count() > 40, "volume {v}: digits visible as holes");
        }
    }

    #[test]
    fn template_not_connected_is_dimmed() {
        let on = render_styled(Some(7), true, Style::Template);
        let off = render_styled(Some(7), false, Style::Template);
        let max = |p: &[u8]| p.chunks_exact(4).map(|q| q[3]).max().unwrap();
        assert_eq!(max(&on), 255);
        assert!(max(&off) < 130, "lighter when not connected");
        assert_eq!(off, render_styled(None, false, Style::Template), "and shows a dash, not a volume");
    }

    #[test]
    fn connecting_dots_fill_in_one_by_one() {
        for style in [Style::Template, Style::Colour] {
            let frames: Vec<Vec<u8>> = (0..CONNECTING_FRAMES).map(|f| render_connecting(f, style)).collect();
            for w in frames.windows(2) {
                assert_ne!(w[0], w[1], "{style:?}: every frame differs");
            }
            assert_eq!(render_connecting(9, style), frames[3], "out-of-range frames show all three dots");
            // the middle of the first dot: waiting in frame 0, filled from frame 1
            let dot = (((SIZE / 2) * SIZE + SIZE / 2 - 16) * 4) as usize;
            match style {
                Style::Template => {
                    assert!(frames[0][dot + 3] > 100, "waiting dot is only a faint hole");
                    assert_eq!(frames[1][dot + 3], 0, "filled dot is a clear hole");
                }
                Style::Colour => {
                    assert!(frames[0][dot] < 200, "waiting dot is greyish");
                    assert_eq!(&frames[1][dot..dot + 3], &[255, 255, 255], "filled dot is white");
                }
            }
            // no dash left over in the middle (between the dots)
            let between = (((SIZE / 2) * SIZE + SIZE / 2 - 8) * 4) as usize;
            assert_eq!(frames[3][between + 3], 255, "{style:?}: solid between the dots");
        }
    }

    #[test]
    fn the_platform_picks_its_style() {
        assert_eq!(PLATFORM_STYLE == Style::Template, cfg!(target_os = "macos"));
    }

    #[test]
    fn different_volumes_and_states_look_different() {
        assert_ne!(render(Some(7), true), render(Some(8), true));
        assert_ne!(render(Some(7), true), render(Some(7), false));
        assert_eq!(render(Some(7), false), render(None, false), "disconnected ignores the volume");
    }
}
