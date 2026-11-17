//! The look of the app: fonts, light and dark colours, text styles and spacing, plus a few small
//! helpers for icons and buttons.
//!
//! Text is Inter (Regular, Medium and `SemiBold`, bundled), icons are Material Symbols, and
//! egui's own fonts stay behind them as fallbacks for any character Inter does not have. The
//! colours live in [`tokens`] so every screen draws from one palette.

mod controls;
mod fading;
mod mark;
mod tokens;

pub use controls::{PILL_HEIGHT, chip, filter_chip, menu_body, pill_buttons, pill_frame, switch};
pub use fading::{fade_opacity, pill_chip};
pub use mark::app_mark;
pub use tokens::*;

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
/// Corner radius of buttons, icon buttons and rows.
pub const CONTROL_RADIUS: u8 = 6;
/// Side of an icon button.
pub const ICON_BUTTON: f32 = 32.0;
/// Height of a chip.
pub const CHIP_HEIGHT: f32 = 28.0;
/// Corner radius of the search field and other text fields.
pub const FIELD_RADIUS: u8 = 8;
/// Space between grid tiles.
pub const GAP: f32 = 2.0;
/// Inner margin of the central panel (horizontal, vertical).
pub const PANEL_MARGIN: (i8, i8) = (16, 12);

/// The text style for date headers in the grids.
pub fn section() -> TextStyle {
    TextStyle::Name("section".into())
}

/// The `SemiBold` family (also used by headings).
pub fn semibold_family() -> FontFamily {
    FontFamily::Name("Inter-SemiBold".into())
}

/// The Medium (500) family, for emphasis.
pub fn medium_family() -> FontFamily {
    FontFamily::Name("Inter-Medium".into())
}

/// `text` in Medium.
pub fn medium(text: impl Into<String>) -> RichText {
    RichText::new(text).family(medium_family())
}

/// `text` in `SemiBold`.
pub fn semibold(text: impl Into<String>) -> RichText {
    RichText::new(text).family(semibold_family())
}

fn fonts() -> FontDefinitions {
    let mut defs = FontDefinitions::default();
    defs.font_data.insert(
        "Inter".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../../assets/fonts/Inter-Regular.ttf"
        ))),
    );
    defs.font_data.insert(
        "Inter-SemiBold".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../../assets/fonts/Inter-SemiBold.ttf"
        ))),
    );
    defs.font_data.insert(
        "Inter-Medium".into(),
        Arc::new(FontData::from_static(include_bytes!(
            "../../../assets/fonts/Inter-Medium.ttf"
        ))),
    );
    let proportional = defs.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "Inter".into());
    let fallbacks: Vec<String> = proportional.clone();
    for (family, face) in [
        (medium_family(), "Inter-Medium"),
        (semibold_family(), "Inter-SemiBold"),
    ] {
        let mut names = vec![face.to_string()];
        names.extend(fallbacks.iter().cloned());
        defs.families.insert(family, names);
    }
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
        (TextStyle::Body, FontId::new(13.0, inter.clone())),
        (TextStyle::Button, FontId::new(13.0, inter.clone())),
        (TextStyle::Heading, FontId::new(20.0, semibold_family())),
        (TextStyle::Small, FontId::new(12.0, inter)),
        (
            TextStyle::Monospace,
            FontId::new(12.0, FontFamily::Monospace),
        ),
        (section(), FontId::new(14.0, semibold_family())),
    ]
    .into();
    let s = &mut style.spacing;
    s.item_spacing = Vec2::new(8.0, 6.0);
    s.button_padding = Vec2::new(12.0, 6.0);
    s.interact_size = Vec2::new(32.0, 28.0);
    s.menu_margin = Margin::same(6);
    s.window_margin = Margin::same(16);
    s.slider_width = 120.0;
    s.slider_rail_height = 6.0;
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
        // Ctrl +/- would change the zoom behind the Text size setting's back.
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
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

/// An icon of `icon_size` followed, `gap` points later, by `label` in the button font. Both are
/// centred on the line. Without a `label` the job holds the icon alone.
pub fn icon_label_job(
    ui: &egui::Ui,
    icon: MaterialIcon,
    icon_size: f32,
    icon_color: Color32,
    label: Option<(&str, Color32)>,
    gap: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        icon.codepoint,
        0.0,
        egui::TextFormat {
            font_id: icon_font_id(icon, icon_size),
            color: icon_color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    if let Some((text, color)) = label {
        job.append(
            text,
            gap,
            egui::TextFormat {
                font_id: TextStyle::Button.resolve(ui.style()),
                color,
                valign: egui::Align::Center,
                ..Default::default()
            },
        );
    }
    job
}

/// A button with an icon in front of its label, named `label` for assistive technology.
pub fn icon_label_button(ui: &mut egui::Ui, icon: MaterialIcon, label: &str) -> Response {
    let color = ui.visuals().widgets.inactive.fg_stroke.color;
    let job = icon_label_job(ui, icon, 18.0, color, Some((label, color)), 6.0);
    let r = ui.add(egui::Button::new(job));
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    r
}

/// `color` moved `amount` (0 to 1) of the way toward black, still opaque when it was.
pub fn darken(color: Color32, amount: f32) -> Color32 {
    let keep = 1.0 - amount;
    let dim = |c: u8| (f32::from(c) * keep).round() as u8;
    Color32::from_rgba_unmultiplied(dim(color.r()), dim(color.g()), dim(color.b()), color.a())
}

/// The one main button of a screen: accent fill, no border, a little darker on hover.
/// Named `label` for assistive technology.
pub fn primary_button(ui: &mut egui::Ui, label: &str) -> Response {
    let pal = Palette::of(ui.ctx());
    filled_button(ui, label, pal.accent, pal.onaccent)
}

/// Like [`primary_button`] but in the warning colour, for the button that confirms something
/// that cannot be undone.
pub fn danger_button(ui: &mut egui::Ui, label: &str) -> Response {
    let pal = Palette::of(ui.ctx());
    filled_button(ui, label, pal.danger, pal.onaccent)
}

/// A 32 point button filled with `fill`, a little darker on hover, named `label`.
fn filled_button(ui: &mut egui::Ui, label: &str, fill: Color32, text: Color32) -> Response {
    let r = ui
        .scope(|ui| {
            let w = &mut ui.visuals_mut().widgets;
            w.inactive.weak_bg_fill = fill;
            w.hovered.weak_bg_fill = darken(fill, 0.12);
            w.active.weak_bg_fill = darken(fill, 0.22);
            let b = egui::Button::new(medium(label).color(text)).min_size(Vec2::new(0.0, 32.0));
            ui.add(b)
        })
        .inner
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    r
}

/// An icon-only button (32 points) that opens a menu, named `label` for assistive technology.
pub fn icon_only_menu<R>(
    ui: &mut egui::Ui,
    icon: MaterialIcon,
    label: &str,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> Response {
    let r = ui
        .scope(|ui| {
            let spacing = ui.spacing_mut();
            spacing.interact_size = Vec2::splat(ICON_BUTTON);
            spacing.button_padding = Vec2::ZERO;
            ui.menu_button(icon_text(icon, 20.0), |ui| {
                controls::menu_body(ui);
                add_contents(ui)
            })
            .response
        })
        .inner
        .on_hover_text(label)
        .on_hover_cursor(egui::CursorIcon::PointingHand);
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
    let color = ui.visuals().widgets.inactive.fg_stroke.color;
    let job = icon_label_job(ui, icon, 18.0, color, Some((label, color)), 6.0);
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

/// A rounded card frame on the surface colour.
pub fn card(ui: &egui::Ui) -> egui::Frame {
    egui::Frame::new()
        .fill(ui.visuals().window_fill)
        .stroke(Stroke::new(1.0, ui.visuals().window_stroke.color))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(16))
}

/// The name of the platform's command key as shown in tooltips.
pub fn command_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "⌘"
    } else {
        "Ctrl"
    }
}

/// A square icon button of the top bar: 17 point icon, a fill on hover, a tooltip with the
/// action and its shortcut. Named `label` for assistive technology.
pub fn top_bar_button(
    ui: &mut egui::Ui,
    icon: MaterialIcon,
    label: &str,
    tooltip: &str,
) -> Response {
    square_icon_button(ui, icon, label, tooltip, IconLook::bar())
}

/// How a top bar toggle looks when it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToggleLook {
    /// The icon alone changes (a filled heart): drawn in the text colour.
    Strong,
    /// The button stays pressed (a panel is open): text colour on a filled square.
    Pressed,
}

/// A top bar icon button that can be on: a heart that is filled, a panel that is open. Named
/// `label`; assistive technology also hears whether it is `on`.
pub fn top_bar_toggle(
    ui: &mut egui::Ui,
    icon: MaterialIcon,
    label: &str,
    tooltip: &str,
    on: Option<ToggleLook>,
) -> Response {
    let look = IconLook {
        strong: on.is_some(),
        pressed: on == Some(ToggleLook::Pressed),
        ..IconLook::bar()
    };
    let r = square_icon_button(ui, icon, label, tooltip, look);
    r.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::Button, ui.is_enabled(), on.is_some(), label)
    });
    r
}

/// A smaller icon button (24 points) for section headers; the tooltip is the label.
pub fn small_icon_button(ui: &mut egui::Ui, icon: MaterialIcon, label: &str) -> Response {
    let look = IconLook {
        side: 24.0,
        icon_size: 16.0,
        ..IconLook::bar()
    };
    square_icon_button(ui, icon, label, label, look)
}

/// Size and state of a square icon button.
#[derive(Clone, Copy)]
struct IconLook {
    side: f32,
    icon_size: f32,
    /// Drawn in the text colour instead of the quieter one.
    strong: bool,
    /// Filled even without the pointer.
    pressed: bool,
}

impl IconLook {
    const fn bar() -> Self {
        Self {
            side: ICON_BUTTON,
            icon_size: 17.0,
            strong: false,
            pressed: false,
        }
    }
}

fn square_icon_button(
    ui: &mut egui::Ui,
    icon: MaterialIcon,
    label: &str,
    tooltip: &str,
    look: IconLook,
) -> Response {
    let pal = Palette::of(ui.ctx());
    let (rect, r) = ui.allocate_exact_size(Vec2::splat(look.side), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let fill = if r.is_pointer_button_down_on() {
            pal.sel
        } else if r.hovered() || r.has_focus() {
            pal.hover
        } else if look.pressed {
            pal.sel
        } else {
            Color32::TRANSPARENT
        };
        ui.painter()
            .rect_filled(rect, CornerRadius::same(CONTROL_RADIUS), fill);
        let color = if !ui.is_enabled() {
            pal.ring
        } else if look.strong {
            pal.text
        } else {
            pal.text2
        };
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            icon.codepoint,
            icon_font_id(icon, look.icon_size),
            color,
        );
    }
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, ui.is_enabled(), label));
    r.on_hover_text(tooltip)
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_darkened_accent_stays_opaque_and_gets_darker() {
        for accent in [Palette::light().accent, Palette::dark().accent] {
            for amount in [0.12, 0.22] {
                let d = darken(accent, amount);
                assert_eq!(d.a(), 255);
                assert!(d.r() < accent.r() && d.g() < accent.g() && d.b() < accent.b());
            }
            assert!(darken(accent, 0.22).g() < darken(accent, 0.12).g());
        }
    }
}
