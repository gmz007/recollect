//! The Help sheet: a centered panel like Settings, with a list of topics on the left and the
//! chosen topic on the right, all in plain words.

use super::App;
use super::sidebar::plain_row;
use super::style::{self, Palette, icons};
use egui::{CornerRadius, Margin, RichText, Stroke, Vec2};
use egui_material_icons::MaterialIcon;

/// Corner radius of the sheet.
const SHEET_RADIUS: u8 = 12;

/// The topics of the sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum HelpTopic {
    #[default]
    GettingStarted,
    Searching,
    Organizing,
    Shortcuts,
    Privacy,
}

impl HelpTopic {
    const ALL: [HelpTopic; 5] = [
        HelpTopic::GettingStarted,
        HelpTopic::Searching,
        HelpTopic::Organizing,
        HelpTopic::Shortcuts,
        HelpTopic::Privacy,
    ];

    pub(super) fn name(self) -> &'static str {
        match self {
            HelpTopic::GettingStarted => "Getting started",
            HelpTopic::Searching => "Searching",
            HelpTopic::Organizing => "Organizing",
            HelpTopic::Shortcuts => "Keyboard shortcuts",
            HelpTopic::Privacy => "Privacy",
        }
    }

    fn icon(self) -> MaterialIcon {
        match self {
            HelpTopic::GettingStarted => icons::ICON_ROCKET_LAUNCH,
            HelpTopic::Searching => icons::ICON_SEARCH,
            HelpTopic::Organizing => icons::ICON_PHOTO_ALBUM,
            HelpTopic::Shortcuts => icons::ICON_KEYBOARD,
            HelpTopic::Privacy => icons::ICON_LOCK,
        }
    }
}

/// A short bold title with a paragraph under it.
fn point(ui: &mut egui::Ui, title: &str, text: &str) {
    ui.add_space(8.0);
    ui.label(style::semibold(title).size(15.0));
    ui.add_space(2.0);
    ui.label(text);
}

/// A numbered step.
fn step(ui: &mut egui::Ui, n: usize, title: &str, text: &str) {
    let pal = Palette::of(ui.ctx());
    ui.add_space(8.0);
    ui.horizontal_top(|ui| {
        ui.label(style::semibold(format!("{n}")).size(18.0).color(pal.accent));
        ui.vertical(|ui| {
            ui.label(style::semibold(title).size(15.0));
            ui.add_space(2.0);
            ui.label(text);
        });
    });
}

/// The topic list on the left of the sheet.
fn help_nav(ui: &mut egui::Ui, pal: &Palette, current: &mut HelpTopic, nav_w: f32, h: f32) {
    egui::Frame::new()
        .fill(pal.side)
        .corner_radius(CornerRadius {
            nw: SHEET_RADIUS,
            sw: SHEET_RADIUS,
            ne: 0,
            se: 0,
        })
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width(nav_w - 24.0);
                ui.set_height(h - 24.0);
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.add_space(4.0);
                ui.label(style::semibold("Help").size(18.0));
                ui.add_space(10.0);
                for t in HelpTopic::ALL {
                    let (_, r) = plain_row(
                        ui,
                        t.icon(),
                        t.name(),
                        &format!("{} help", t.name()),
                        None,
                        *current == t,
                    );
                    if r.clicked() {
                        *current = t;
                    }
                }
            });
        });
}

/// The text of one topic with its title and close button; true when the button was clicked.
fn help_content(ui: &mut egui::Ui, topic: HelpTopic, width: f32, height: f32) -> bool {
    let mut close = false;
    egui::Frame::new()
        .inner_margin(Margin::symmetric(20, 14))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.set_width(width);
                ui.set_height(height);
                ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(topic.name()).text_style(egui::TextStyle::Heading));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if style::icon_button_framed(ui, icons::ICON_CLOSE, "Close help").clicked()
                        {
                            close = true;
                        }
                    });
                });
                ui.add_space(6.0);
                ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                egui::ScrollArea::vertical()
                    .id_salt(("help-scroll", topic as u8))
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_max_width(ui.available_width() - 8.0);
                        match topic {
                            HelpTopic::GettingStarted => getting_started(ui),
                            HelpTopic::Searching => searching(ui),
                            HelpTopic::Organizing => organizing(ui),
                            HelpTopic::Shortcuts => shortcuts(ui),
                            HelpTopic::Privacy => privacy(ui),
                        }
                        ui.add_space(24.0);
                    });
            });
        });
    close
}

impl App {
    /// The sheet itself. Closed by its × button, by Esc and by a click outside it.
    pub(super) fn help_sheet(&mut self, ctx: &egui::Context) {
        let pal = Palette::of(ctx);
        let screen = ctx.content_rect().size();
        let w = (screen.x - 48.0).clamp(380.0, 720.0);
        let h = (screen.y - 48.0).clamp(320.0, 580.0);
        let nav_w = if w < 520.0 { 170.0 } else { 224.0 };
        let frame = egui::Frame::new()
            .fill(pal.pop)
            .stroke(Stroke::new(1.0, pal.line))
            .corner_radius(CornerRadius::same(SHEET_RADIUS))
            .shadow(ctx.global_style().visuals.popup_shadow)
            .inner_margin(Margin::ZERO);
        let mut close = false;
        let out = egui::Modal::new(egui::Id::unique("help-sheet"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(w);
                ui.set_height(h);
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                ui.horizontal_top(|ui| {
                    help_nav(ui, &pal, &mut self.dialogs.help_topic, nav_w, h);
                    close = help_content(ui, self.dialogs.help_topic, w - nav_w - 40.0, h - 28.0);
                });
            });
        if close || out.backdrop_response.clicked() {
            self.dialogs.help_open = false;
        }
    }
}

fn getting_started(ui: &mut egui::Ui) {
    step(
        ui,
        1,
        "Add your photo folders",
        "Click the + next to Folders in the left panel and choose where your photos and \
         videos are. Recollect only reads them; it never changes or moves them.",
    );
    step(
        ui,
        2,
        "Turn on smart search",
        "Click Turn on when Recollect offers it (or open Settings and choose Smart search, \
         then Download and turn on). It is a one-time download, and afterwards \
         search works without the internet.",
    );
    step(
        ui,
        3,
        "Search or browse",
        "Type in the search box at the top, or scroll through Photos by date. Click any photo \
         to open it.",
    );
}

fn searching(ui: &mut egui::Ui) {
    point(
        ui,
        "Describe what you remember",
        "Say what is in the photo the way you would tell a friend: “kids on the beach”, \
         “birthday cake”, “snow” or “a red car”. Short descriptions of things you can see \
         work best. Click the search box to see more examples and your recent searches.",
    );
    point(
        ui,
        "Narrow the results",
        "Next to the search box, All, Photos and Videos show one kind at a time. After a \
         search, When limits the results to a year, and Sort puts the best matches, the \
         newest or the oldest first.",
    );
    point(
        ui,
        "Moments in videos",
        "For a video, the result says “at 1:23”, which is the moment that matches your \
         words. Open the video to see the matching moments and play from there.",
    );
}

fn organizing(ui: &mut egui::Ui) {
    point(
        ui,
        "Favorites",
        "Click the heart on a photo, or press F, to mark it as a favorite. All your favorites \
         are together under Favorites in the left panel.",
    );
    point(
        ui,
        "Albums",
        "Click the + next to Albums to make a new album. Then open it and click Add photos, \
         or select photos anywhere and choose Add to album. Photos stay in their folders; \
         an album only collects them.",
    );
    point(
        ui,
        "Selecting several photos",
        "Hover a photo and click the circle in its corner. Then click more photos to add \
         them. Click Cancel, or press Esc, to stop selecting.",
    );
}

fn shortcuts(ui: &mut egui::Ui) {
    // One grid for all sections, so the columns line up.
    egui::Grid::new("shortcuts")
        .num_columns(2)
        .spacing([16.0, 6.0])
        .show(ui, |ui| {
            for (n, (section, keys)) in SHORTCUTS.iter().enumerate() {
                if n > 0 {
                    ui.label("");
                    ui.end_row();
                }
                ui.label(style::semibold(*section).size(15.0));
                ui.end_row();
                for (key, what) in *keys {
                    ui.label(style::semibold(*key));
                    ui.label(*what);
                    ui.end_row();
                }
            }
        });
}

fn privacy(ui: &mut egui::Ui) {
    point(
        ui,
        "Everything stays on this computer",
        "Your photos, videos and searches are never sent anywhere. The only time Recollect \
         uses the internet is the one-time download for smart search.",
    );
    point(
        ui,
        "Your files are safe",
        "Recollect only reads your photos and videos. It never changes, moves or deletes them.",
    );
    point(
        ui,
        "Recent searches",
        "Your recent searches are remembered on this computer only. You can clear them from \
         the search box.",
    );
}

/// Keys and what they do, in sections, for the help sheet.
const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Browsing",
        &[
            ("Arrow keys", "Move between photos"),
            ("Enter", "Open the photo you are on"),
            ("Esc", "Go back, or clear the selection"),
            ("Tab, then Enter", "Jump along the dates beside the photos"),
            ("Ctrl+F (Cmd on Mac)", "Go to the search box"),
            ("F1", "Show this help"),
            ("Ctrl+, (Cmd on Mac)", "Open Settings"),
        ],
    ),
    (
        "Photo size",
        &[
            (
                "Ctrl+= and Ctrl+- (Cmd on Mac)",
                "Make photos bigger or smaller",
            ),
            ("Ctrl+0 (Cmd on Mac)", "Back to the usual photo size"),
            (
                "Pinch, or Ctrl+scroll (Cmd on Mac)",
                "Change the photo size smoothly",
            ),
        ],
    ),
    (
        "Selecting",
        &[
            ("Ctrl+click (Cmd on Mac)", "Select several photos"),
            ("Shift+click", "Select everything in between"),
            ("Ctrl+A (Cmd on Mac)", "Select everything"),
            ("F", "Mark as favorite, or remove the mark"),
        ],
    ),
    (
        "Viewing a photo",
        &[
            ("Left and right arrows", "Previous and next photo"),
            ("Home and End", "First and last photo"),
            ("I", "Show or hide photo details"),
            ("Z, or double-click", "Zoom in and out"),
            ("Pinch", "Zoom in and out"),
            ("Esc", "Back to the photos"),
        ],
    ),
];
