//! The look of the app: fonts, light and dark colours, text styles and spacing, plus a few small
//! helpers for icons and icon buttons.
//!
//! Text is Inter (Regular and SemiBold, bundled), icons are Material Symbols, and egui's own
//! fonts stay behind them as fallbacks for any character Inter does not have. The colours and
//! sizes live here so every screen draws from one palette.

use crate::config::{TextSize, Theme};
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Response,
    RichText, Stroke, TextStyle, Vec2, epaint::text::FontInsert,
};
use egui_material_icons::MaterialIcon;
use std::sync::Arc;

pub use egui_material_icons::icons;

/// Corner radius of cards, windows and menus.
pub const CARD_RADIUS: u8 = 10;
/// Corner radius of buttons and fields.
pub const CONTROL_RADIUS: u8 = 8;
/// Corner radius of photo tiles.
pub const TILE_RADIUS: u8 = 6;

/// The colours of one theme.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub background: Color32,
    pub surface: Color32,
    pub raised: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub accent: Color32,
    /// Text and icons drawn on top of the accent colour.
    pub on_accent: Color32,
    pub border: Color32,
    pub error: Color32,
    pub favorite: Color32,
}

impl Palette {
    pub fn dark() -> Self {
        Self {
            dark: true,
            background: Color32::from_rgb(0x12, 0x12, 0x14),
            surface: Color32::from_rgb(0x1A, 0x1A, 0x1E),
            raised: Color32::from_rgb(0x23, 0x23, 0x29),
            text: Color32::from_rgb(0xED, 0xED, 0xF0),
            text_secondary: Color32::from_rgb(0xA1, 0xA1, 0xAA),
            accent: Color32::from_rgb(0x7C, 0x95, 0xFF),
            on_accent: Color32::from_rgb(0x0E, 0x10, 0x20),
            border: Color32::from_rgb(0x2E, 0x2E, 0x36),
            error: Color32::from_rgb(0xFF, 0x8A, 0x80),
            favorite: Color32::from_rgb(0xFF, 0xC8, 0x28),
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            background: Color32::from_rgb(0xF7, 0xF7, 0xF9),
            surface: Color32::WHITE,
            raised: Color32::from_rgb(0xEF, 0xEF, 0xF3),
            text: Color32::from_rgb(0x18, 0x18, 0x1B),
            text_secondary: Color32::from_rgb(0x52, 0x52, 0x5B),
            accent: Color32::from_rgb(0x3B, 0x5B, 0xDB),
            on_accent: Color32::WHITE,
            border: Color32::from_rgb(0xDD, 0xDD, 0xE4),
            error: Color32::from_rgb(0xC9, 0x2A, 0x2A),
            favorite: Color32::from_rgb(0xF5, 0xA6, 0x00),
        }
    }

    /// The palette of the theme `ui` is drawn with.
    pub fn of(ctx: &egui::Context) -> Self {
        if ctx.global_style().visuals.dark_mode {
            Self::dark()
        } else {
            Self::light()
        }
    }

    /// `self.text` mixed towards the background by `t` (0 = text, 1 = background).
    fn mix_text(&self, base: Color32, t: f32) -> Color32 {
        base.lerp_to_gamma(self.text, t)
    }

    fn visuals(&self) -> egui::Visuals {
        let mut v = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        let control = CornerRadius::same(CONTROL_RADIUS);
        v.panel_fill = self.background;
        v.window_fill = self.surface;
        v.window_stroke = Stroke::new(1.0, self.border);
        v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
        v.menu_corner_radius = CornerRadius::same(CONTROL_RADIUS);
        v.extreme_bg_color = self.raised;
        v.faint_bg_color = self.surface;
        v.code_bg_color = self.raised;
        v.hyperlink_color = self.accent;
        v.warn_fg_color = self.favorite;
        v.error_fg_color = self.error;
        v.selection.bg_fill = self.accent;
        v.selection.stroke = Stroke::new(1.0, self.on_accent);
        v.text_cursor.stroke = Stroke::new(2.0, self.accent);
        v.window_shadow = egui::Shadow {
            offset: [0, 6],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(if self.dark { 120 } else { 40 }),
        };
        v.popup_shadow = v.window_shadow;
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.surface;
        w.noninteractive.weak_bg_fill = self.surface;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        w.noninteractive.corner_radius = control;
        w.inactive.bg_fill = self.raised;
        w.inactive.weak_bg_fill = self.raised;
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);
        w.inactive.corner_radius = control;
        let hover = self.mix_text(self.raised, 0.08);
        w.hovered.bg_fill = hover;
        w.hovered.weak_bg_fill = hover;
        w.hovered.bg_stroke = Stroke::NONE;
        w.hovered.fg_stroke = Stroke::new(1.5, self.text);
        w.hovered.corner_radius = control;
        let active = self.mix_text(self.raised, 0.16);
        w.active.bg_fill = active;
        w.active.weak_bg_fill = active;
        w.active.bg_stroke = Stroke::NONE;
        w.active.fg_stroke = Stroke::new(1.5, self.text);
        w.active.corner_radius = control;
        w.open.bg_fill = hover;
        w.open.weak_bg_fill = hover;
        w.open.bg_stroke = Stroke::NONE;
        w.open.fg_stroke = Stroke::new(1.0, self.text);
        w.open.corner_radius = control;
        v
    }
}

/// The text style for date headers in the grids.
pub fn section() -> TextStyle {
    TextStyle::Name("section".into())
}

/// The SemiBold family (also used by headings).
pub fn semibold_family() -> FontFamily {
    FontFamily::Name("Inter-SemiBold".into())
}

/// `text` in SemiBold.
pub fn semibold(text: impl Into<String>) -> RichText {
    RichText::new(text).family(semibold_family())
}

fn fonts() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    defs.font_data.insert(
        "Inter".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/Inter-Regular.ttf"
        ))),
    );
    defs.font_data.insert(
        "Inter-SemiBold".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../assets/fonts/Inter-SemiBold.ttf"
        ))),
    );
    let proportional = defs.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "Inter".into());
    let mut bold = vec!["Inter-SemiBold".to_string()];
    bold.extend(proportional.iter().cloned());
    defs.families.insert(semibold_family(), bold);
    defs
}

fn icon_font() -> FontInsert {
    egui_material_icons::font_insert()
}

fn icon_font_outlined() -> FontInsert {
    egui_material_icons::font_insert_outlined()
}

/// Installs the fonts. Safe to call every frame: nothing happens once they are in place.
fn install_fonts(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.add_font(icon_font());
    ctx.add_font(icon_font_outlined());
}

fn apply_text_styles(style: &mut egui::Style) {
    let inter = FontFamily::Proportional;
    style.text_styles = [
        (TextStyle::Body, FontId::new(14.5, inter.clone())),
        (TextStyle::Button, FontId::new(14.5, inter.clone())),
        (TextStyle::Heading, FontId::new(22.0, semibold_family())),
        (TextStyle::Small, FontId::new(12.5, inter)),
        (
            TextStyle::Monospace,
            FontId::new(13.0, FontFamily::Monospace),
        ),
        (section(), FontId::new(17.0, semibold_family())),
    ]
    .into();
    let s = &mut style.spacing;
    s.item_spacing = Vec2::new(8.0, 6.0);
    s.button_padding = Vec2::new(12.0, 6.0);
    s.interact_size = Vec2::new(36.0, 32.0);
    s.menu_margin = Margin::same(8);
    s.window_margin = Margin::same(16);
    s.slider_width = 120.0;
    s.indent = 16.0;
    style.interaction.tooltip_delay = 0.35;
}

/// What was applied last (kept in the egui context, so every context is set up once).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Applied {
    theme: Theme,
    text_size: TextSize,
}

/// Applies the fonts, colours, text styles and zoom for `theme` and `text_size`. The first call
/// on a context installs the fonts; later calls with unchanged settings do nothing. Returns true
/// when something changed (a `Ui` created earlier in the same frame still has the old style).
pub fn apply(ctx: &egui::Context, theme: Theme, text_size: TextSize) -> bool {
    let key = egui::Id::unique("recollect-style");
    let now = Applied { theme, text_size };
    let last: Option<Applied> = ctx.data(|d| d.get_temp(key));
    if last == Some(now) {
        return false;
    }
    if last.is_none() {
        install_fonts(ctx);
        ctx.set_visuals_of(egui::Theme::Dark, Palette::dark().visuals());
        ctx.set_visuals_of(egui::Theme::Light, Palette::light().visuals());
        ctx.all_styles_mut(apply_text_styles);
    }
    ctx.set_theme(match theme {
        Theme::System => egui::ThemePreference::System,
        Theme::Light => egui::ThemePreference::Light,
        Theme::Dark => egui::ThemePreference::Dark,
    });
    ctx.set_zoom_factor(text_size.zoom());
    ctx.data_mut(|d| d.insert_temp(key, now));
    true
}

/// A [`FontId`] for an icon of `size`.
pub fn icon_font_id(icon: MaterialIcon, size: f32) -> FontId {
    FontId::new(size, icon.font_family())
}

/// An icon as rich text, `size` points high.
pub fn icon_text(icon: MaterialIcon, size: f32) -> RichText {
    icon.rich_text().size(size)
}

/// A button with an icon in front of its label, named `label` for assistive technology.
pub fn icon_label_button(ui: &mut egui::Ui, icon: MaterialIcon, label: &str) -> Response {
    let mut job = egui::text::LayoutJob::default();
    let color = ui.visuals().widgets.inactive.fg_stroke.color;
    job.append(
        icon.codepoint,
        0.0,
        egui::TextFormat {
            font_id: icon_font_id(icon, 18.0),
            color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    job.append(
        label,
        6.0,
        egui::TextFormat {
            font_id: TextStyle::Button.resolve(ui.style()),
            color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    let r = ui.add(egui::Button::new(job));
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    r
}

/// A frameless icon-only button with a hover tooltip that also names it for assistive
/// technology.
pub fn icon_button(ui: &mut egui::Ui, icon: MaterialIcon, label: &str) -> Response {
    let r = ui
        .add(egui::Button::new(icon_text(icon, 20.0)).frame(false))
        .on_hover_text(label);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    r
}

/// Like [`icon_button`] with a filled background.
pub fn icon_button_framed(ui: &mut egui::Ui, icon: MaterialIcon, label: &str) -> Response {
    let r = ui
        .add(egui::Button::new(icon_text(icon, 20.0)))
        .on_hover_text(label);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    r
}

/// A menu button with an icon and a label, named `label`.
pub fn icon_menu_button<R>(
    ui: &mut egui::Ui,
    icon: MaterialIcon,
    label: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Response {
    let mut job = egui::text::LayoutJob::default();
    let color = ui.visuals().widgets.inactive.fg_stroke.color;
    job.append(
        icon.codepoint,
        0.0,
        egui::TextFormat {
            font_id: icon_font_id(icon, 18.0),
            color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    job.append(
        label,
        6.0,
        egui::TextFormat {
            font_id: TextStyle::Button.resolve(ui.style()),
            color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    let r = ui.menu_button(job, add_contents).response;
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    r
}

/// A text with a drop-down arrow, for menu buttons.
pub fn dropdown_job(ui: &egui::Ui, label: &str) -> egui::text::LayoutJob {
    let color = ui.visuals().widgets.inactive.fg_stroke.color;
    let mut job = egui::text::LayoutJob::default();
    job.append(
        label,
        0.0,
        egui::TextFormat {
            font_id: TextStyle::Button.resolve(ui.style()),
            color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    job.append(
        icons::ICON_ARROW_DROP_DOWN.codepoint,
        2.0,
        egui::TextFormat {
            font_id: icon_font_id(icons::ICON_ARROW_DROP_DOWN, 20.0),
            color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    job
}

/// A drop-down "chip": `text` with an arrow that opens a menu. The chip is named `accessible`
/// for assistive technology.
pub fn chip<R>(
    ui: &mut egui::Ui,
    accessible: &str,
    text: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Response {
    let job = dropdown_job(ui, text);
    let r = ui.menu_button(job, add_contents).response;
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), accessible));
    r
}

/// A frameless icon button for toolbars: tooltip `tooltip`, named `label` for assistive
/// technology, drawn in `color` (default: the normal text colour) and highlighted when
/// `selected`.
pub fn toolbar_icon(
    ui: &mut egui::Ui,
    icon: MaterialIcon,
    label: &str,
    tooltip: &str,
    color: Option<Color32>,
    selected: bool,
) -> Response {
    let mut text = icon_text(icon, 22.0);
    if let Some(c) = color {
        text = text.color(c);
    }
    let b = egui::Button::new(text)
        .frame(selected)
        .selected(selected)
        .min_size(Vec2::splat(34.0));
    let r = ui.add(b).on_hover_text(tooltip);
    r.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), selected, label)
    });
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A rounded card frame on the surface colour.
pub fn card(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().window_fill)
        .stroke(Stroke::new(1.0, ui.visuals().window_stroke.color))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(16))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn palettes_follow_the_spec() {
        let d = Palette::dark();
        assert_eq!(d.background, Color32::from_rgb(0x12, 0x12, 0x14));
        assert_eq!(d.accent, Color32::from_rgb(0x7C, 0x95, 0xFF));
        let l = Palette::light();
        assert_eq!(l.surface, Color32::WHITE);
        assert_eq!(l.accent, Color32::from_rgb(0x3B, 0x5B, 0xDB));
    }
}
