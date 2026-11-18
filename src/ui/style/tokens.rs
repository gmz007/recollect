//! The colours of the two themes, one named value per role. Screens read colours from here
//! (`Palette::of(ctx)`) instead of writing colour literals.
#![expect(
    clippy::unreadable_literal,
    reason = "colour codes are written as hex triplets, as designers write them"
)]

use super::{CARD_RADIUS, CONTROL_RADIUS};
use egui::{Color32, CornerRadius, Stroke};

/// The colours of one theme.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Main content background.
    pub bg: Color32,
    /// Sidebar and quiet panels.
    pub side: Color32,
    /// Primary text.
    pub text: Color32,
    /// Body copy in explanations and toolbar icons.
    pub text2: Color32,
    /// Labels and secondary lines.
    pub muted: Color32,
    /// Hairlines and dividers.
    pub line: Color32,
    /// Search field, secondary buttons and chips.
    pub field: Color32,
    /// Hover fill for rows and icon buttons.
    pub hover: Color32,
    /// Selected sidebar row and pressed icon button.
    pub sel: Color32,
    /// Popovers and menus.
    pub pop: Color32,
    /// Primary button, links, progress and focus.
    pub accent: Color32,
    /// Text on the accent colour.
    pub onaccent: Color32,
    /// Selection bar, confirmation and active chip.
    pub pill: Color32,
    /// Text on the pill.
    pub pilltext: Color32,
    /// Progress track, input borders and off dots.
    pub ring: Color32,
    /// Destructive menu items.
    pub danger: Color32,
    /// Loading placeholder tiles.
    pub skel: Color32,
    /// Selected-photo cell and tip banner.
    pub tint: Color32,
}

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// The "on" colour of switches, the same in both themes.
pub const SWITCH_ON: Color32 = rgb(0x3366E6);
/// White overlays on photos (94 %).
pub const OVERLAY_WHITE: Color32 = Color32::from_rgba_unmultiplied_const(255, 255, 255, 240);
/// Text on a white overlay: dark in both themes.
pub const OVERLAY_WHITE_TEXT: Color32 = rgb(0x1C1C1B);
/// Black overlays on photos (55 %).
pub const OVERLAY_BLACK: Color32 = Color32::from_rgba_unmultiplied_const(0, 0, 0, 140);
/// The Settings window and the backdrop behind it in the dark theme.
pub const SETTINGS_WINDOW_DARK: Color32 = rgb(0x1A1A19);
pub const SETTINGS_BACKDROP_DARK: Color32 = rgb(0x0C0C0C);

/// Surfaces and text of the photo viewer in one theme.
#[derive(Debug, Clone, Copy)]
pub struct ViewerColors {
    pub stage: Color32,
    pub controls: Color32,
    pub controls_text: Color32,
    pub bar: Color32,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "the popovers take their fill from the theme's popup colour"
        )
    )]
    pub menu: Color32,
}

impl Palette {
    pub const fn dark() -> Self {
        Self {
            dark: true,
            bg: rgb(0x161616),
            side: rgb(0x1D1D1C),
            text: rgb(0xEDEDEB),
            text2: rgb(0xBDBDB8),
            muted: rgb(0x9C9C96),
            line: rgb(0x272726),
            field: rgb(0x262625),
            hover: rgb(0x2E2E2C),
            sel: rgb(0x353533),
            pop: rgb(0x262625),
            accent: rgb(0x86A4FF),
            onaccent: rgb(0x0F1424),
            pill: rgb(0xEDEDEB),
            pilltext: rgb(0x161616),
            ring: rgb(0x3A3A38),
            danger: rgb(0xFF9080),
            skel: rgb(0x242423),
            tint: rgb(0x1C2233),
        }
    }

    pub const fn light() -> Self {
        Self {
            dark: false,
            bg: rgb(0xFFFFFF),
            side: rgb(0xF3F3F1),
            text: rgb(0x1C1C1B),
            text2: rgb(0x55554F),
            muted: rgb(0x6E6E69),
            line: rgb(0xEFEFEC),
            field: rgb(0xF1F1EF),
            hover: rgb(0xE9E9E6),
            sel: rgb(0xE4E4E1),
            pop: rgb(0xFFFFFF),
            accent: rgb(0x3366E6),
            onaccent: rgb(0xFFFFFF),
            pill: rgb(0x1C1C1B),
            pilltext: rgb(0xFFFFFF),
            ring: rgb(0xDCDCD8),
            danger: rgb(0xB42318),
            skel: rgb(0xECECE9),
            tint: rgb(0xE4ECFF),
        }
    }

    /// The palette of the theme `ctx` is drawn with.
    pub fn of(ctx: &egui::Context) -> Self {
        if ctx.global_style().visuals.dark_mode {
            Self::dark()
        } else {
            Self::light()
        }
    }

    /// The viewer's surfaces for this theme.
    pub const fn viewer(&self) -> ViewerColors {
        if self.dark {
            ViewerColors {
                stage: rgb(0x0F0F0F),
                controls: Color32::from_rgba_unmultiplied_const(30, 30, 30, 230),
                controls_text: rgb(0xF2F2F0),
                bar: rgb(0x161616),
                menu: rgb(0x262625),
            }
        } else {
            ViewerColors {
                stage: rgb(0xF3F3F1),
                controls: Color32::from_rgba_unmultiplied_const(255, 255, 255, 240),
                controls_text: rgb(0x1C1C1B),
                bar: rgb(0xFFFFFF),
                menu: rgb(0xFFFFFF),
            }
        }
    }

    /// The egui visuals for this theme.
    pub(super) fn visuals(&self) -> egui::Visuals {
        let mut v = if self.dark {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        };
        self.window_visuals(&mut v);
        self.widget_visuals(&mut v.widgets);
        v
    }

    /// Backgrounds, popups, links, selection and shadows.
    fn window_visuals(&self, v: &mut egui::Visuals) {
        v.panel_fill = self.bg;
        v.window_fill = self.pop;
        v.window_stroke = Stroke::new(1.0, self.line);
        v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
        v.menu_corner_radius = CornerRadius::same(CARD_RADIUS);
        v.extreme_bg_color = self.field;
        v.faint_bg_color = self.side;
        v.code_bg_color = self.field;
        v.hyperlink_color = self.accent;
        v.warn_fg_color = self.text2;
        v.error_fg_color = self.danger;
        v.selection.bg_fill = self.sel;
        v.selection.stroke = Stroke::new(1.0, self.text);
        v.text_cursor.stroke = Stroke::new(2.0, self.accent);
        v.window_shadow = egui::Shadow {
            offset: [0, 6],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(if self.dark { 120 } else { 32 }),
        };
        v.popup_shadow = v.window_shadow;
    }

    /// Fills and corners of buttons and other widgets in each state. No borders: state shows
    /// as a fill that gets slightly darker (lighter in the dark theme) on hover and press.
    fn widget_visuals(&self, w: &mut egui::style::Widgets) {
        let control = CornerRadius::same(CONTROL_RADIUS);
        let states = [
            (&mut w.noninteractive, self.side, self.line, 1.0),
            (&mut w.inactive, self.field, Color32::TRANSPARENT, 1.0),
            (&mut w.hovered, self.hover, Color32::TRANSPARENT, 1.5),
            (&mut w.active, self.sel, Color32::TRANSPARENT, 1.5),
            (&mut w.open, self.hover, Color32::TRANSPARENT, 1.0),
        ];
        for (widget, fill, stroke, fg_width) in states {
            widget.bg_fill = fill;
            widget.weak_bg_fill = fill;
            widget.bg_stroke = Stroke::new(1.0, stroke);
            widget.fg_stroke = Stroke::new(fg_width, self.text);
            widget.corner_radius = control;
        }
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(c: Color32) -> u32 {
        (u32::from(c.r()) << 16) | (u32::from(c.g()) << 8) | u32::from(c.b())
    }

    fn values(p: &Palette) -> [u32; 18] {
        [
            p.bg, p.side, p.text, p.text2, p.muted, p.line, p.field, p.hover, p.sel, p.pop,
            p.accent, p.onaccent, p.pill, p.pilltext, p.ring, p.danger, p.skel, p.tint,
        ]
        .map(hex)
    }

    #[test]
    fn light_tokens_match_the_spec() {
        assert_eq!(
            values(&Palette::light()),
            [
                0xFFFFFF, 0xF3F3F1, 0x1C1C1B, 0x55554F, 0x6E6E69, 0xEFEFEC, 0xF1F1EF, 0xE9E9E6,
                0xE4E4E1, 0xFFFFFF, 0x3366E6, 0xFFFFFF, 0x1C1C1B, 0xFFFFFF, 0xDCDCD8, 0xB42318,
                0xECECE9, 0xE4ECFF
            ]
        );
    }

    #[test]
    fn dark_tokens_match_the_spec() {
        assert_eq!(
            values(&Palette::dark()),
            [
                0x161616, 0x1D1D1C, 0xEDEDEB, 0xBDBDB8, 0x9C9C96, 0x272726, 0x262625, 0x2E2E2C,
                0x353533, 0x262625, 0x86A4FF, 0x0F1424, 0xEDEDEB, 0x161616, 0x3A3A38, 0xFF9080,
                0x242423, 0x1C2233
            ]
        );
    }

    #[test]
    fn fixed_and_viewer_colours_match_the_spec() {
        assert_eq!(hex(SWITCH_ON), 0x3366E6);
        assert_eq!(OVERLAY_WHITE.a(), 240);
        assert_eq!(OVERLAY_BLACK.a(), 140);
        let (l, d) = (Palette::light().viewer(), Palette::dark().viewer());
        assert_eq!((hex(l.stage), hex(d.stage)), (0xF3F3F1, 0x0F0F0F));
        assert_eq!(
            (hex(l.controls_text), hex(d.controls_text)),
            (0x1C1C1B, 0xF2F2F0)
        );
        assert_eq!((l.controls.a(), d.controls.a()), (240, 230));
        assert_eq!((hex(l.bar), hex(d.bar)), (0xFFFFFF, 0x161616));
        assert_eq!((hex(l.menu), hex(d.menu)), (0xFFFFFF, 0x262625));
        assert_eq!(hex(SETTINGS_WINDOW_DARK), 0x1A1A19);
        assert_eq!(hex(SETTINGS_BACKDROP_DARK), 0x0C0C0C);
    }
}
