//! The photo mosaic beside the first-run screens: 16 bundled photographs in a static 4 by 4
//! grid, cropped to fill their cells. The light set is shown in the light theme and the dark
//! set in the dark one. The JPEGs are decoded once on a background thread and become textures
//! on the first frame after; the textures live only as long as the [`Mosaic`] does.

use crate::ui::style::Palette;
use egui::{
    ColorImage, Context, CornerRadius, Rect, Sense, TextureHandle, TextureOptions, Ui, Vec2, pos2,
};
use std::sync::mpsc::{Receiver, channel};

const COLUMNS: usize = 4;
const ROWS: usize = 4;
/// Gap between photos, and between the photos and the edge.
const GAP: f32 = 4.0;
const PHOTO_RADIUS: u8 = 6;
/// The longest side photos are shrunk to before they become textures: plenty for a cell.
const MAX_SIDE: u32 = 480;

static LIGHT: [&[u8]; COLUMNS * ROWS] = [
    include_bytes!("../../../assets/welcome/light-01.jpg"),
    include_bytes!("../../../assets/welcome/light-02.jpg"),
    include_bytes!("../../../assets/welcome/light-03.jpg"),
    include_bytes!("../../../assets/welcome/light-04.jpg"),
    include_bytes!("../../../assets/welcome/light-05.jpg"),
    include_bytes!("../../../assets/welcome/light-06.jpg"),
    include_bytes!("../../../assets/welcome/light-07.jpg"),
    include_bytes!("../../../assets/welcome/light-08.jpg"),
    include_bytes!("../../../assets/welcome/light-09.jpg"),
    include_bytes!("../../../assets/welcome/light-10.jpg"),
    include_bytes!("../../../assets/welcome/light-11.jpg"),
    include_bytes!("../../../assets/welcome/light-12.jpg"),
    include_bytes!("../../../assets/welcome/light-13.jpg"),
    include_bytes!("../../../assets/welcome/light-14.jpg"),
    include_bytes!("../../../assets/welcome/light-15.jpg"),
    include_bytes!("../../../assets/welcome/light-16.jpg"),
];

static DARK: [&[u8]; COLUMNS * ROWS] = [
    include_bytes!("../../../assets/welcome/dark-01.jpg"),
    include_bytes!("../../../assets/welcome/dark-02.jpg"),
    include_bytes!("../../../assets/welcome/dark-03.jpg"),
    include_bytes!("../../../assets/welcome/dark-04.jpg"),
    include_bytes!("../../../assets/welcome/dark-05.jpg"),
    include_bytes!("../../../assets/welcome/dark-06.jpg"),
    include_bytes!("../../../assets/welcome/dark-07.jpg"),
    include_bytes!("../../../assets/welcome/dark-08.jpg"),
    include_bytes!("../../../assets/welcome/dark-09.jpg"),
    include_bytes!("../../../assets/welcome/dark-10.jpg"),
    include_bytes!("../../../assets/welcome/dark-11.jpg"),
    include_bytes!("../../../assets/welcome/dark-12.jpg"),
    include_bytes!("../../../assets/welcome/dark-13.jpg"),
    include_bytes!("../../../assets/welcome/dark-14.jpg"),
    include_bytes!("../../../assets/welcome/dark-15.jpg"),
    include_bytes!("../../../assets/welcome/dark-16.jpg"),
];

/// Which of the two photo sets is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::ui) enum PhotoSet {
    Light,
    Dark,
}

impl PhotoSet {
    pub(super) fn for_theme(dark: bool) -> Self {
        if dark { Self::Dark } else { Self::Light }
    }

    fn photos(self) -> &'static [&'static [u8]] {
        match self {
            Self::Light => &LIGHT,
            Self::Dark => &DARK,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

enum State {
    /// Being decoded; `None` marks a photo that could not be.
    Decoding(Receiver<Vec<Option<ColorImage>>>),
    Ready(Vec<Option<TextureHandle>>),
}

pub(super) struct Mosaic {
    set: PhotoSet,
    state: State,
}

/// Decodes and shrinks one bundled photo.
fn decode(bytes: &[u8]) -> Option<ColorImage> {
    let img = image::load_from_memory(bytes).ok()?;
    let small = img.thumbnail(MAX_SIDE, MAX_SIDE).to_rgb8();
    let size = [small.width() as usize, small.height() as usize];
    Some(ColorImage::from_rgb(size, small.as_raw()))
}

impl Mosaic {
    /// Starts decoding `set` in the background; the UI repaints when it is done.
    pub(super) fn new(ctx: &Context, set: PhotoSet) -> Self {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let images = set.photos().iter().map(|b| decode(b)).collect();
            // The receiver is gone when the welcome was closed meanwhile; nothing to do then.
            let _ = tx.send(images);
            ctx.request_repaint();
        });
        Self {
            set,
            state: State::Decoding(rx),
        }
    }

    pub(super) fn set(&self) -> PhotoSet {
        self.set
    }

    /// Turns finished decodes into textures.
    fn poll(&mut self, ctx: &Context) {
        let State::Decoding(rx) = &self.state else {
            return;
        };
        let Ok(images) = rx.try_recv() else {
            return;
        };
        let name = self.set.name();
        let textures = images
            .into_iter()
            .enumerate()
            .map(|(i, img)| {
                img.map(|img| {
                    ctx.load_texture(format!("welcome-{name}-{i}"), img, TextureOptions::LINEAR)
                })
            })
            .collect();
        self.state = State::Ready(textures);
    }

    /// Paints the grid over all the space `ui` has left.
    pub(super) fn show(&mut self, ui: &mut Ui) {
        self.poll(ui.ctx());
        let pal = Palette::of(ui.ctx());
        let (area, _) = ui.allocate_exact_size(ui.available_size(), Sense::hover());
        ui.painter().rect_filled(area, 0.0, pal.side);
        let cell = Vec2::new(
            (area.width() - GAP * (COLUMNS + 1) as f32) / COLUMNS as f32,
            (area.height() - GAP * (ROWS + 1) as f32) / ROWS as f32,
        );
        for i in 0..COLUMNS * ROWS {
            let (col, row) = ((i % COLUMNS) as f32, (i / COLUMNS) as f32);
            let min = area.min + Vec2::new(GAP + col * (cell.x + GAP), GAP + row * (cell.y + GAP));
            let rect = Rect::from_min_size(min, cell);
            match &self.state {
                State::Ready(t) if t[i].is_some() => {
                    let tex = t[i].as_ref().expect("checked by the guard");
                    egui::Image::new((tex.id(), tex.size_vec2()))
                        .uv(cover_uv(tex.size_vec2(), cell))
                        .corner_radius(CornerRadius::same(PHOTO_RADIUS))
                        .paint_at(ui, rect);
                }
                _ => {
                    let radius = CornerRadius::same(PHOTO_RADIUS);
                    ui.painter().rect_filled(rect, radius, pal.field);
                }
            }
        }
    }
}

/// The part of an image of size `image`, centred, that fills a cell of size `cell` without
/// stretching.
fn cover_uv(image: Vec2, cell: Vec2) -> Rect {
    let image_aspect = image.x / image.y;
    let cell_aspect = cell.x / cell.y;
    if image_aspect > cell_aspect {
        let w = cell_aspect / image_aspect;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2(f32::midpoint(1.0, w), 1.0))
    } else {
        let h = image_aspect / cell_aspect;
        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, f32::midpoint(1.0, h)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_wide_image_is_cropped_at_the_sides_and_a_tall_one_top_and_bottom() {
        let wide = cover_uv(Vec2::new(200.0, 100.0), Vec2::new(100.0, 100.0));
        assert_eq!(
            (wide.min.x, wide.max.x, wide.min.y, wide.max.y),
            (0.25, 0.75, 0.0, 1.0)
        );
        let tall = cover_uv(Vec2::new(100.0, 200.0), Vec2::new(100.0, 100.0));
        assert_eq!(
            (tall.min.y, tall.max.y, tall.min.x, tall.max.x),
            (0.25, 0.75, 0.0, 1.0)
        );
    }

    #[test]
    fn every_bundled_photo_decodes_to_a_bounded_size() {
        for set in [PhotoSet::Light, PhotoSet::Dark] {
            for bytes in set.photos() {
                let img = decode(bytes).expect("bundled photo decodes");
                assert!(img.size.iter().all(|&s| s <= MAX_SIDE as usize));
            }
        }
    }
}
