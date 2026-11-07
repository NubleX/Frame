//! The address bar. It is drawn here on the host, so a page cannot paint over
//! it. Anything outside ASCII shows as `?`, which also makes look-alike
//! letters in a hostile address visible.

use font8x8::legacy::BASIC_LEGACY;

pub const BAR_HEIGHT: usize = 36;

const SCALE: usize = 2;
const GLYPH: usize = 8 * SCALE;
const BUTTON: usize = 36;
const FIELD_LEFT: usize = BUTTON * 3 + 6;
const FIELD_MARGIN: usize = 4;
const TEXT_LEFT: usize = FIELD_LEFT + 8;
/// Too narrow to hold the bar, so it is left blank.
const MIN_WIDTH: usize = 200;

const BACKGROUND: u32 = 0x07_05_0c;
const FIELD: u32 = 0x1c_14_30;
const FIELD_EDITING: u32 = 0x2e_1f_4d;
const TEXT: u32 = 0xe9_e3_f5;
const ACCENT: u32 = 0xe8_79_b8;

/// What a click on the bar means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Back,
    Forward,
    Reload,
    Address,
}

pub fn hit(x: usize) -> Hit {
    match x / BUTTON {
        0 => Hit::Back,
        1 => Hit::Forward,
        2 => Hit::Reload,
        _ => Hit::Address,
    }
}

fn fill(canvas: &mut [u32], width: usize, area: [usize; 4], color: u32) {
    let [left, top, right, bottom] = area;
    for y in top..bottom {
        canvas[y * width + left..y * width + right].fill(color);
    }
}

fn draw_text(canvas: &mut [u32], width: usize, at: (usize, usize), text: &str, color: u32) {
    for (index, character) in text.chars().enumerate() {
        let code = if character.is_ascii() {
            character as usize
        } else {
            '?' as usize
        };
        for (row, bits) in BASIC_LEGACY[code].iter().enumerate() {
            for column in 0..8 {
                if bits >> column & 1 == 1 {
                    let x = at.0 + index * GLYPH + column * SCALE;
                    let y = at.1 + row * SCALE;
                    fill(canvas, width, [x, y, x + SCALE, y + SCALE], color);
                }
            }
        }
    }
}

/// The part of `text` that fits: the end while typing, the start otherwise.
fn visible(text: &str, editing: bool, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        text.to_owned()
    } else if editing {
        text.chars().skip(count - max_chars).collect()
    } else {
        text.chars().take(max_chars).collect()
    }
}

pub fn draw(canvas: &mut [u32], width: usize, text: &str, editing: bool) {
    fill(canvas, width, [0, 0, width, BAR_HEIGHT], BACKGROUND);
    if width < MIN_WIDTH {
        return;
    }
    let top = (BAR_HEIGHT - GLYPH) / 2;
    for (index, label) in ["<", ">", "R"].iter().enumerate() {
        let at = (index * BUTTON + (BUTTON - GLYPH) / 2, top);
        draw_text(canvas, width, at, label, ACCENT);
    }
    let right = width - 6;
    let field = if editing { FIELD_EDITING } else { FIELD };
    let area = [FIELD_LEFT, FIELD_MARGIN, right, BAR_HEIGHT - FIELD_MARGIN];
    fill(canvas, width, area, field);
    let max_chars = (right - TEXT_LEFT - 8) / GLYPH;
    let shown = visible(text, editing, max_chars);
    draw_text(canvas, width, (TEXT_LEFT, top), &shown, TEXT);
    if editing {
        let caret = TEXT_LEFT + shown.chars().count() * GLYPH;
        fill(
            canvas,
            width,
            [caret, top, caret + SCALE, top + GLYPH],
            ACCENT,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: usize = 1280;

    #[test]
    fn clicks_land_on_the_right_control() {
        assert_eq!(hit(0), Hit::Back);
        assert_eq!(hit(BUTTON), Hit::Forward);
        assert_eq!(hit(BUTTON * 2), Hit::Reload);
        assert_eq!(hit(BUTTON * 3), Hit::Address);
        assert_eq!(hit(WIDTH - 1), Hit::Address);
    }

    #[test]
    fn long_text_never_draws_outside_the_field() {
        for width in [MIN_WIDTH, 201, 640, WIDTH, 3840] {
            let mut canvas = vec![0; width * BAR_HEIGHT];
            let long = "x".repeat(5000);
            draw(&mut canvas, width, &long, true);
            draw(&mut canvas, width, &long, false);
        }
    }

    #[test]
    fn a_tiny_window_is_left_blank() {
        let mut canvas = vec![0; 10 * BAR_HEIGHT];
        draw(&mut canvas, 10, "text", true);
    }

    #[test]
    fn non_ascii_text_cannot_index_outside_the_font() {
        let mut canvas = vec![0; WIDTH * BAR_HEIGHT];
        draw(&mut canvas, WIDTH, "\u{430}pple.com \u{1f600}", false);
    }
}
