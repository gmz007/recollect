//! The timeline ruler: a column of dashes at the right edge of a grid that replaces the scroll
//! bar. Pressing or dragging on it scrolls the grid, the dash of the current position is
//! marked, and a small chip names the date there while the pointer is on the ruler or the grid
//! is scrolling.
//!
//! Photos and the other views grouped by day use a ruler of months ([`scale::TimeScale`]);
//! search results and ungrouped lists use the plain scroll position. The positions of the dashes
//! are worked out when the layout changes, never per item per frame.

mod panel;
mod scale;

pub use scale::{Mapping, Month, TimeScale};

use crate::ui::style::{Palette, fade_opacity, pill_chip};
use egui::{Align2, CursorIcon, Id, Rect, Sense, Stroke, Vec2};
use scale::{Dash, MIN_SPACING};
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

/// Width of the ruler beside Photos, albums and folders, and beside search results.
pub(super) const WIDTH: f32 = 56.0;
pub(super) const WIDTH_SEARCH: f32 = 40.0;
/// Space above and below the dashes, and to the right of them.
const PAD_Y: f32 = 16.0;
const PAD_RIGHT: f32 = 6.0;
/// How far the dashes near the pointer grow, and over how many dashes that fades out.
const GROW: f32 = 8.0;
const GROW_FALLOFF: f32 = 1.2;
/// Dashes this close to the focus hide their year label.
const LABEL_HIDDEN_WITHIN: f32 = 5.0;
/// Opacity of the ruler until the pointer is on it.
const REST_OPACITY: f32 = 0.7;
/// How long the date chip stays after the last move, and how much of that it spends fading.
const CHIP_FOR: Duration = Duration::from_millis(900);
const CHIP_FADE: Duration = Duration::from_millis(300);
/// Distance of the chip's right edge from the right edge of the ruler.
const CHIP_INSET: f32 = 28.0;

/// What a grid remembers about its ruler.
#[derive(Default)]
pub(super) struct RulerState {
    mapping: Option<Mapping>,
    /// What the mapping was built for (see [`key_of`]).
    model_key: Option<u64>,
    dashes: Vec<Dash>,
    dashes_for: Option<(u64, i32, i32)>,
    revision: u64,
    /// Where the grid is scrolled, how far it can scroll and how tall its view is, as of the
    /// last time it was drawn.
    offset: f32,
    max: f32,
    measured: bool,
    last_offset: Option<f32>,
    /// When the pointer or the scrolling last moved something.
    active_at: Option<Instant>,
    was_on_ruler: bool,
}

/// The month of the entry at the point `frac` (0 the first, 1 the last) of a list of `len`
/// entries, for rulers that follow the scroll position. `taken_at` gives the capture time of
/// an entry.
pub(super) fn month_at_fraction(
    len: usize,
    frac: f32,
    taken_at: impl Fn(usize) -> Option<i64>,
) -> Option<Month> {
    let last = len.checked_sub(1)?;
    let at = (frac.clamp(0.0, 1.0) * last as f32).round() as usize;
    Month::of_timestamp(taken_at(at)?)
}

/// A fingerprint of everything a mapping was built from.
pub(super) fn key_of(parts: &impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut h);
    h.finish()
}

impl RulerState {
    /// Whether the mapping must be rebuilt for `key`.
    pub(super) fn needs_mapping(&self, key: u64) -> bool {
        self.model_key != Some(key)
    }

    pub(super) fn set_mapping(&mut self, key: u64, mapping: Mapping) {
        self.mapping = Some(mapping);
        self.model_key = Some(key);
        self.revision += 1;
    }

    /// Records where the grid is scrolled (call after drawing it).
    pub(super) fn observe(&mut self, offset: f32, max: f32) {
        self.offset = offset;
        self.max = max.max(0.0);
        self.measured = true;
    }

    /// True once the grid is known to fit its view, so there is nothing to scroll.
    pub(super) fn fits(&self) -> bool {
        self.measured && self.max <= 1.0
    }

    /// The scroll offset the grid reported last.
    pub(in crate::ui) fn offset(&self) -> f32 {
        self.offset
    }

    fn ensure_dashes(&mut self, height: f32) {
        let key = (self.revision, height as i32, self.max as i32);
        if self.dashes_for != Some(key) {
            let max = self.max;
            self.dashes = self
                .mapping
                .as_ref()
                .map_or_else(Vec::new, |m| scale::dashes(m, height, max));
            self.dashes_for = Some(key);
        }
    }
}

/// What the pointer is doing to the ruler this frame.
struct Pointer {
    /// Position on the ruler (0 top, 1 bottom) and in points, if the pointer is on it or
    /// dragging.
    at: Option<(f32, f32)>,
    pressed: bool,
}

/// Draws the ruler in `ui` and returns the scroll offset the user asked for, if any (a press or
/// drag, the mouse wheel over the ruler, or Enter on a focused dash). `date_at` names the date
/// at a point of the ruler (0 top, 1 bottom) for rulers that follow the scroll position.
pub(super) fn show(
    ui: &mut egui::Ui,
    st: &mut RulerState,
    date_at: &dyn Fn(f32) -> Option<Month>,
) -> Option<f32> {
    let rect = ui.max_rect();
    let track = Rect::from_min_max(
        egui::pos2(rect.left(), rect.top() + PAD_Y),
        egui::pos2(rect.right() - PAD_RIGHT, rect.bottom() - PAD_Y),
    );
    if st.mapping.is_none() || track.height() < MIN_SPACING * 4.0 {
        return None;
    }
    st.ensure_dashes(track.height());
    let id = ui.make_persistent_id("timeline-ruler");
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let pointer = read_pointer(ui, &resp, track);
    let mut target = request_from_pointer(ui, st, &resp, &pointer);
    let focus_jump = stops(ui, st, rect, track, id);
    target = focus_jump.or(target);
    let hot = resp.hovered() || pointer.pressed || stop_focused(ui.ctx());
    let opacity = REST_OPACITY
        + (1.0 - REST_OPACITY) * ui.ctx().animate_bool_with_time(id.with("hot"), hot, 0.12);
    let marker = marker_frac(st, &pointer);
    let focus = pointer.at.map_or(marker, |(f, _)| f);
    paint_dashes(ui, st, track, focus, marker, opacity);
    chip(ui, st, rect, track, date_at, &pointer, resp.hovered());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(if pointer.pressed {
            CursorIcon::Grabbing
        } else {
            CursorIcon::Grab
        });
    }
    target
}

fn read_pointer(ui: &egui::Ui, resp: &egui::Response, track: Rect) -> Pointer {
    // A click that starts and ends within one frame counts as a press too.
    let pressed = resp.is_pointer_button_down_on() || resp.clicked();
    let y = if pressed {
        ui.ctx().pointer_latest_pos().map(|p| p.y)
    } else {
        resp.hover_pos().map(|p| p.y)
    };
    Pointer {
        at: y.map(|y| (((y - track.top()) / track.height()).clamp(0.0, 1.0), y)),
        pressed,
    }
}

/// The offset a press, drag or mouse wheel on the ruler asks for.
fn request_from_pointer(
    ui: &egui::Ui,
    st: &RulerState,
    resp: &egui::Response,
    pointer: &Pointer,
) -> Option<f32> {
    let mapping = st.mapping.as_ref()?;
    if let (true, Some((frac, _))) = (pointer.pressed, pointer.at) {
        return Some(mapping.offset_at(frac, st.max));
    }
    let wheel = if resp.hovered() {
        ui.input(|i| i.smooth_scroll_delta.y)
    } else {
        0.0
    };
    (wheel != 0.0).then(|| (st.offset - wheel).clamp(0.0, st.max))
}

/// The keyboard stops: Tab reaches them, Enter scrolls to the one in focus.
fn stops(ui: &mut egui::Ui, st: &mut RulerState, rect: Rect, track: Rect, id: Id) -> Option<f32> {
    let mut jump = None;
    let mut ids = Vec::with_capacity(st.dashes.len());
    let pal = Palette::of(ui.ctx());
    for (i, dash) in st.dashes.iter().enumerate() {
        let Some(stop) = &dash.stop else { continue };
        let y = track.top() + dash.frac * track.height();
        let area =
            Rect::from_center_size(egui::pos2(rect.center().x, y), Vec2::new(rect.width(), 8.0));
        let r = ui.interact(
            area,
            id.with(("stop", i)),
            Sense::focusable_noninteractive(),
        );
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &stop.label));
        ids.push(r.id);
        if r.has_focus() {
            let to = egui::pos2(track.right(), y);
            ui.painter().hline(
                to.x - 28.0..=to.x,
                ui.painter().round_to_pixel_center(y),
                Stroke::new(2.0, pal.accent),
            );
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
                let max = st.max;
                jump = st.mapping.as_ref().map(|m| m.offset_at(stop.target, max));
            }
        }
    }
    ui.ctx().data_mut(|d| d.insert_temp(stop_ids_key(), ids));
    jump
}

fn stop_ids_key() -> Id {
    Id::NULL.with("ruler-stop-ids")
}

/// Whether the widget with keyboard focus is one of the ruler's stops, as of the last time a
/// ruler was drawn. Asking egui, not remembering a flag, keeps this right when the ruler of
/// one grid goes away while another is shown.
pub(super) fn stop_focused(ctx: &egui::Context) -> bool {
    let Some(focus) = ctx.memory(egui::Memory::focused) else {
        return false;
    };
    ctx.data(|d| d.get_temp::<Vec<Id>>(stop_ids_key()))
        .is_some_and(|ids| ids.contains(&focus))
}

/// Forgets the stops (no ruler is shown).
pub(super) fn clear_stops(ctx: &egui::Context) {
    ctx.data_mut(|d| d.remove_temp::<Vec<Id>>(stop_ids_key()));
}

/// Where the current position is: under the pointer while it presses, else where the grid is.
fn marker_frac(st: &RulerState, pointer: &Pointer) -> f32 {
    match (pointer.pressed, pointer.at, &st.mapping) {
        (true, Some((f, _)), _) => f,
        (_, _, Some(m)) => m.fraction_at(st.offset, st.max),
        _ => 0.0,
    }
}

/// Length of a dash `dist` dashes from the focus.
fn grown(base: f32, dist: f32) -> f32 {
    base + (GROW - dist * GROW_FALLOFF).max(0.0)
}

fn paint_dashes(
    ui: &egui::Ui,
    st: &RulerState,
    track: Rect,
    focus_frac: f32,
    marker: f32,
    opacity: f32,
) {
    let pal = Palette::of(ui.ctx());
    let painter = ui.painter();
    let n = st.dashes.len();
    let spacing = (track.height() / n as f32).max(2.0);
    let (at, focus_at) = (
        scale::nearest(&st.dashes, marker),
        scale::nearest(&st.dashes, focus_frac),
    );
    let focus_y = track.top() + focus_frac * track.height();
    let font = egui::FontId::proportional(10.0);
    for (i, d) in st.dashes.iter().enumerate() {
        let y = track.top() + d.frac * track.height();
        let dist = (y - focus_y).abs() / spacing;
        let len = grown(d.size.length(), dist);
        let (color, alpha) = if i == at {
            (pal.text, 1.0)
        } else if i == focus_at && dist < 1.0 {
            (pal.text, 0.9)
        } else if d.size == scale::DashSize::Year {
            (pal.muted, 0.8)
        } else {
            (pal.muted, 0.5)
        };
        let y_px = painter.round_to_pixel_center(y);
        painter.hline(
            track.right() - len..=track.right(),
            y_px,
            Stroke::new(1.0, color.gamma_multiply(alpha * opacity)),
        );
        if let Some(year) = d.year.filter(|_| dist > LABEL_HIDDEN_WITHIN) {
            painter.text(
                egui::pos2(track.right() - len - 6.0, y_px),
                Align2::RIGHT_CENTER,
                year.to_string(),
                font.clone(),
                pal.muted.gamma_multiply(opacity),
            );
        }
    }
}

/// The date chip beside the ruler while the pointer is on it or the grid is moving.
fn chip(
    ui: &egui::Ui,
    st: &mut RulerState,
    rect: Rect,
    track: Rect,
    date_at: &dyn Fn(f32) -> Option<Month>,
    pointer: &Pointer,
    hovered: bool,
) {
    let moved = st.last_offset.is_some_and(|o| (o - st.offset).abs() > 0.5);
    st.last_offset = Some(st.offset);
    let on_ruler = (hovered || pointer.pressed) && pointer.at.is_some();
    // The chip lingers from the moment the pointer leaves, not from the last frame it was on
    // (frames stop while the pointer rests).
    if on_ruler || moved || st.was_on_ruler {
        st.active_at = Some(Instant::now());
    }
    st.was_on_ruler = on_ruler;
    // While the pointer rests on the ruler the chip stays as it is: nothing needs redrawing.
    let alpha = if on_ruler {
        1.0
    } else {
        st.active_at.map_or(0.0, |t| {
            fade_opacity(ui.ctx(), t.elapsed(), CHIP_FOR, CHIP_FADE)
        })
    };
    if alpha <= 0.0 {
        return;
    }
    let frac = pointer
        .at
        .filter(|_| on_ruler)
        .map_or_else(|| marker_frac(st, pointer), |(f, _)| f);
    let Some(text) = chip_text(st, frac, date_at) else {
        return;
    };
    let y = track.top() + frac * track.height();
    let pos = egui::pos2(rect.right() - CHIP_INSET, y);
    pill_chip(
        ui.ctx(),
        ui.make_persistent_id("timeline-ruler-chip"),
        pos,
        Align2::RIGHT_CENTER,
        alpha,
        |ui, pal| {
            ui.label(egui::RichText::new(text).size(11.5).color(pal.pilltext));
        },
    );
}

/// What the chip says for the point `frac` of the ruler.
fn chip_text(st: &RulerState, frac: f32, date_at: &dyn Fn(f32) -> Option<Month>) -> Option<String> {
    match st.mapping.as_ref()? {
        Mapping::Time(t) => Some(t.month_at(frac).name()),
        Mapping::Flat | Mapping::Dated(_) => date_at(frac).map(Month::name),
    }
}
