//! Fits the fixed page into whatever window it is shown in. The guest always
//! renders the same size, so the window never affects what pages see.

pub const PAGE_WIDTH: usize = 1920;
pub const PAGE_HEIGHT: usize = 1080;

/// Where the page sits in a window, below the bar.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub width: usize,
    pub height: usize,
    left: usize,
    top: usize,
    shown_width: usize,
    shown_height: usize,
    scale: f32,
}

impl Layout {
    /// Never enlarges: a window bigger than the page shows it pixel for pixel.
    pub fn new(width: usize, height: usize, bar: usize) -> Self {
        let available = height.saturating_sub(bar);
        let scale = (width as f32 / PAGE_WIDTH as f32)
            .min(available as f32 / PAGE_HEIGHT as f32)
            .clamp(0.01, 1.0);
        let shown_width = ((PAGE_WIDTH as f32 * scale) as usize).min(width);
        let shown_height = ((PAGE_HEIGHT as f32 * scale) as usize).min(available);
        Self {
            width,
            height,
            left: (width - shown_width) / 2,
            top: bar + (available - shown_height) / 2,
            shown_width,
            shown_height,
            scale,
        }
    }

    /// Window position to page position. `None` outside the page.
    pub fn to_page(&self, x: f32, y: f32) -> Option<(u16, u16)> {
        let page_x = (x - self.left as f32) / self.scale;
        let page_y = (y - self.top as f32) / self.scale;
        let inside = (0.0..PAGE_WIDTH as f32).contains(&page_x)
            && (0.0..PAGE_HEIGHT as f32).contains(&page_y);
        inside.then_some((page_x as u16, page_y as u16))
    }
}

/// Blends two pixels. `weight` runs from 0, all `a`, to 256, all `b`.
fn blend(a: u32, b: u32, weight: u32) -> u32 {
    let keep = 256 - weight;
    let red_blue = ((a & 0xff_00ff) * keep + (b & 0xff_00ff) * weight) >> 8;
    let green = ((a & 0x00_ff00) * keep + (b & 0x00_ff00) * weight) >> 8;
    (red_blue & 0xff_00ff) | (green & 0x00_ff00)
}

/// One source position: the two neighbours to read and the mix between them.
fn sample(index: usize, scale: f32, limit: usize) -> (usize, usize, u32) {
    let position = ((index as f32 + 0.5) / scale - 0.5).max(0.0);
    let low = (position as usize).min(limit - 1);
    let high = (low + 1).min(limit - 1);
    (low, high, ((position - low as f32) * 256.0) as u32)
}

/// Draws the page into the window buffer with bilinear filtering, so small
/// text stays readable instead of losing whole rows of pixels.
pub fn draw_page(page: &[u32], window: &mut [u32], layout: &Layout) {
    let columns: Vec<_> = (0..layout.shown_width)
        .map(|x| sample(x, layout.scale, PAGE_WIDTH))
        .collect();
    for y in 0..layout.shown_height {
        let (top, bottom, weight_y) = sample(y, layout.scale, PAGE_HEIGHT);
        let upper = &page[top * PAGE_WIDTH..(top + 1) * PAGE_WIDTH];
        let lower = &page[bottom * PAGE_WIDTH..(bottom + 1) * PAGE_WIDTH];
        let start = (layout.top + y) * layout.width + layout.left;
        let row = &mut window[start..start + layout.shown_width];
        for (pixel, (left, right, weight_x)) in row.iter_mut().zip(&columns) {
            let above = blend(upper[*left], upper[*right], *weight_x);
            let below = blend(lower[*left], lower[*right], *weight_x);
            *pixel = blend(above, below, weight_y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_page_fits_any_window() {
        for (width, height) in [(1360, 742), (1, 1), (200, 37), (3840, 2160), (1280, 756)] {
            let layout = Layout::new(width, height, 36);
            let page = vec![0xff_ffff; PAGE_WIDTH * PAGE_HEIGHT];
            let mut window = vec![0; width * height];
            draw_page(&page, &mut window, &layout);
        }
    }

    #[test]
    fn a_big_window_shows_the_page_pixel_for_pixel() {
        let layout = Layout::new(3840, 2160, 36);
        assert_eq!(layout.scale, 1.0);
        assert_eq!(layout.shown_width, PAGE_WIDTH);
    }

    #[test]
    fn clicks_map_back_to_page_pixels() {
        let layout = Layout::new(1280, 756, 36);
        assert_eq!(layout.to_page(0.0, 36.0), Some((0, 0)));
        assert_eq!(layout.to_page(640.0, 396.0), Some((960, 540)));
        assert_eq!(layout.to_page(640.0, 10.0), None);
    }

    #[test]
    fn blending_stays_inside_each_channel() {
        assert_eq!(blend(0xff_ffff, 0xff_ffff, 128) & 0xff, 0xff);
        assert_eq!(blend(0, 0xff_0000, 256), 0xff_0000);
        assert_eq!(blend(0x00_00ff, 0, 0), 0x00_00ff);
    }
}
