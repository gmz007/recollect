//! The Settings sheet: a centered panel with a list of sections on the left and the chosen
//! section on the right. Changes apply as soon as they are made; only switching the
//! smart-search choice asks first, because it makes Recollect look at every photo again.

use super::albums::modal_window;
use super::sidebar::nav_row;
use super::style::{self, Palette, icons};
use super::timeline::thousands;
use super::{App, folder_labels, platform, prune_thumbnails};
use crate::config::{Settings, TextSize, Theme};
use crate::library::Activity;
use crate::model::registry::{self, ModelSpec, megabytes};
use crate::store::db::Stats;
use egui::{CornerRadius, Margin, Pos2, RichText, Sense, Stroke, Vec2};
use egui_material_icons::MaterialIcon;

/// The sections of the sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(super) enum Section {
    #[default]
    Appearance,
    SmartSearch,
    Library,
    Storage,
    About,
}

impl Section {
    const ALL: [Section; 5] = [
        Section::Appearance,
        Section::SmartSearch,
        Section::Library,
        Section::Storage,
        Section::About,
    ];

    fn name(self) -> &'static str {
        match self {
            Section::Appearance => "Appearance",
            Section::SmartSearch => "Smart search",
            Section::Library => "Library",
            Section::Storage => "Storage",
            Section::About => "About",
        }
    }

    fn icon(self) -> MaterialIcon {
        match self {
            Section::Appearance => icons::ICON_PALETTE,
            Section::SmartSearch => icons::ICON_AUTO_AWESOME,
            Section::Library => icons::ICON_PHOTO_LIBRARY,
            Section::Storage => icons::ICON_STORAGE,
            Section::About => icons::ICON_INFO,
        }
    }
}

/// "Best quality" or "Faster": how a smart-search choice is called in the interface.
pub(super) fn quality_name(spec: &ModelSpec) -> &'static str {
    if spec.id == registry::SIGLIP_B16.id {
        "Best quality"
    } else {
        "Faster"
    }
}

/// One sentence about a smart-search choice, with its download size.
pub(super) fn quality_blurb(spec: &ModelSpec) -> String {
    let mb = spec.size_mb();
    if spec.id == registry::SIGLIP_B16.id {
        format!(
            "Finds photos most reliably. Works best on computers with a graphics card. {mb} MB."
        )
    } else {
        format!("Quicker on older computers. {mb} MB.")
    }
}

/// The smart-search status in plain words.
pub(super) fn smart_search_status(
    activity: &Activity,
    installed: bool,
    spec: &ModelSpec,
) -> String {
    match activity {
        Activity::Downloading { done, total, .. } => format!(
            "Downloading… {} of {} MB",
            thousands(usize::try_from(megabytes(*done)).unwrap_or(usize::MAX)),
            thousands(usize::try_from(megabytes(*total)).unwrap_or(usize::MAX))
        ),
        Activity::Idle if installed => format!("On — {}", quality_name(spec).to_lowercase()),
        Activity::Idle => "Off".to_string(),
        _ if installed => "Getting your photos ready…".to_string(),
        _ => "Off".to_string(),
    }
}

/// "1,204 photos · 37 videos · 1,241 ready for search · 3 couldn't be opened".
pub(super) fn counts_text(stats: &Stats, installed: bool) -> String {
    let mut parts = Vec::new();
    let noun = |n: usize, one: &str, many: &str| {
        format!("{} {}", thousands(n), if n == 1 { one } else { many })
    };
    if stats.images > 0 {
        parts.push(noun(stats.images, "photo", "photos"));
    }
    if stats.videos > 0 {
        parts.push(noun(stats.videos, "video", "videos"));
    }
    if installed && stats.indexed > 0 {
        parts.push(format!("{} ready for search", thousands(stats.indexed)));
    }
    if stats.errors > 0 {
        parts.push(format!("{} couldn't be opened", thousands(stats.errors)));
    }
    parts.join(" · ")
}

/// "512 MB", "1 GB", "1.5 GB".
fn fmt_space(mb: f64) -> String {
    if mb >= 1024.0 {
        let gb = mb / 1024.0;
        if (gb - gb.round()).abs() < 0.05 {
            format!("{} GB", gb.round() as u64)
        } else {
            format!("{gb:.1} GB")
        }
    } else {
        format!("{} MB", mb.round() as u64)
    }
}

/// Reads "2 GB", "512 MB" or a bare number of megabytes.
fn parse_space(text: &str) -> Option<f64> {
    let t = text.trim().to_lowercase();
    if let Some(n) = t.strip_suffix("gb") {
        n.trim().parse::<f64>().ok().map(|g| g * 1024.0)
    } else {
        t.strip_suffix("mb").unwrap_or(&t).trim().parse().ok()
    }
}

/// A row of mutually exclusive choices. Returns the one clicked, if it is not the current.
fn segmented<T: Copy + PartialEq>(ui: &mut egui::Ui, items: &[(T, &str)], current: T) -> Option<T> {
    let pal = Palette::of(ui.ctx());
    let mut picked = None;
    egui::Frame::new()
        .fill(pal.raised)
        .corner_radius(CornerRadius::same(style::CONTROL_RADIUS))
        .inner_margin(Margin::same(2))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (value, label) in items {
                    let selected = *value == current;
                    let font = egui::TextStyle::Body.resolve(ui.style());
                    let g = ui
                        .painter()
                        .layout_no_wrap(label.to_string(), font, pal.text);
                    let (rect, resp) =
                        ui.allocate_exact_size(Vec2::new(g.size().x + 28.0, 30.0), Sense::click());
                    let fg = if selected { pal.on_accent } else { pal.text };
                    if selected {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(7), pal.accent);
                    } else if resp.hovered() {
                        ui.painter()
                            .rect_filled(rect, CornerRadius::same(7), pal.border);
                    }
                    let g = ui.painter().layout_no_wrap(
                        label.to_string(),
                        egui::TextStyle::Body.resolve(ui.style()),
                        fg,
                    );
                    ui.painter().galley(
                        Pos2::new(
                            rect.center().x - g.size().x / 2.0,
                            rect.center().y - g.size().y / 2.0,
                        ),
                        g,
                        fg,
                    );
                    resp.widget_info(|| {
                        egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, *label)
                    });
                    if resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                        && !selected
                    {
                        picked = Some(*value);
                    }
                }
            });
        });
    picked
}

/// A large choice with a title and a description. Returns true when clicked.
pub(super) fn choice_card(
    ui: &mut egui::Ui,
    selected: bool,
    title: &str,
    blurb: &str,
    note: &str,
) -> bool {
    let pal = Palette::of(ui.ctx());
    let width = ui.available_width();
    let text_w = width - 56.0;
    let title_g = ui.painter().layout(
        title.to_string(),
        egui::FontId::new(15.0, style::semibold_family()),
        pal.text,
        text_w,
    );
    let blurb_g = ui.painter().layout(
        blurb.to_string(),
        egui::TextStyle::Body.resolve(ui.style()),
        pal.text_secondary,
        text_w,
    );
    let note_g = (!note.is_empty()).then(|| {
        ui.painter().layout(
            note.to_string(),
            egui::TextStyle::Small.resolve(ui.style()),
            pal.accent,
            text_w,
        )
    });
    let height = 28.0
        + title_g.size().y
        + 4.0
        + blurb_g.size().y
        + note_g.as_ref().map_or(0.0, |g| 4.0 + g.size().y);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    let painter = ui.painter();
    let radius = CornerRadius::same(style::CARD_RADIUS);
    painter.rect_filled(
        rect,
        radius,
        if selected {
            pal.accent.gamma_multiply(0.14)
        } else if resp.hovered() {
            pal.border
        } else {
            pal.raised
        },
    );
    if selected {
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.5, pal.accent),
            egui::StrokeKind::Inside,
        );
    }
    let dot = Pos2::new(
        rect.min.x + 26.0,
        rect.min.y + 14.0 + title_g.size().y / 2.0,
    );
    painter.circle_stroke(
        dot,
        9.0,
        Stroke::new(
            1.5,
            if selected {
                pal.accent
            } else {
                pal.text_secondary
            },
        ),
    );
    if selected {
        painter.circle_filled(dot, 5.0, pal.accent);
    }
    let x = rect.min.x + 48.0;
    let mut y = rect.min.y + 14.0;
    let h = title_g.size().y + 4.0;
    painter.galley(Pos2::new(x, y), title_g, pal.text);
    y += h;
    let h = blurb_g.size().y + 4.0;
    painter.galley(Pos2::new(x, y), blurb_g, pal.text_secondary);
    y += h;
    if let Some(g) = note_g {
        painter.galley(Pos2::new(x, y), g, pal.accent);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, title));
    // The text drawn under the title is what assistive technology reads as its description.
    ui.ctx().accesskit_node_builder(resp.id, |b| {
        b.set_description(blurb);
    });
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

/// A slider whose value is applied when the user lets go (or types a value or uses the
/// keyboard), not on every step of a drag. `current` is the saved value.
fn committed_slider(
    ui: &mut egui::Ui,
    name: &str,
    current: u64,
    range: std::ops::RangeInclusive<u64>,
    step: f64,
    format: fn(f64) -> String,
) -> Option<u64> {
    let key = egui::Id::unique(("settings-slider", name));
    let mut v = ui.data(|d| d.get_temp::<u64>(key)).unwrap_or(current) as f64;
    let slider = egui::Slider::new(&mut v, *range.start() as f64..=*range.end() as f64)
        .step_by(step)
        .custom_formatter(move |n, _| format(n))
        .custom_parser(parse_space);
    ui.spacing_mut().slider_width = 260.0;
    let r = ui.add(slider);
    r.widget_info(|| egui::WidgetInfo::slider(true, v, name));
    if r.changed() {
        ui.data_mut(|d| d.insert_temp(key, v as u64));
    }
    let draft = ui.data(|d| d.get_temp::<u64>(key));
    match draft {
        Some(d) if !r.dragged() => {
            ui.data_mut(|data| data.remove_temp::<u64>(key));
            Some(d)
        }
        _ => None,
    }
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(style::semibold(text).size(15.0));
    ui.add_space(2.0);
}

fn hint(ui: &mut egui::Ui, text: &str) {
    let pal = Palette::of(ui.ctx());
    ui.label(RichText::new(text).small().color(pal.text_secondary));
}

impl App {
    /// Opens Settings on the section the user saw last.
    pub(super) fn open_settings(&mut self) {
        self.settings_open = true;
    }

    /// Opens Settings on `section`.
    pub(super) fn open_settings_at(&mut self, section: Section) {
        self.settings_section = section;
        self.settings_open = true;
    }

    /// Changes the settings `f` touches (and only those) and saves them at once.
    pub(super) fn change_settings(&mut self, f: impl FnOnce(&mut Settings)) {
        match self.lib.update_settings(f) {
            Ok(()) => self.settings = self.lib.settings(),
            Err(e) => self.notify_problem(
                "Couldn't save your settings. Check that your disk isn't full or write-protected, \
                 then try again.",
                format!("{e:#}"),
                None,
            ),
        }
    }

    /// Remembers that the user did (or no longer does) not want the smart-search invitation
    /// above the photos.
    pub(super) fn set_search_prompt_dismissed(&mut self, dismissed: bool) {
        self.change_settings(|s| s.search_prompt_dismissed = dismissed);
    }

    /// The sheet itself. Closed by its × button, by Esc and by a click outside it.
    pub(super) fn settings_sheet(&mut self, ctx: &egui::Context) {
        let pal = Palette::of(ctx);
        let screen = ctx.content_rect().size();
        let w = (screen.x - 48.0).clamp(380.0, 640.0);
        let h = (screen.y - 48.0).clamp(320.0, 560.0);
        let nav_w = if w < 520.0 { 150.0 } else { 188.0 };
        let frame = egui::Frame::new()
            .fill(pal.surface)
            .stroke(Stroke::new(1.0, pal.border))
            .corner_radius(CornerRadius::same(16))
            .shadow(ctx.global_style().visuals.popup_shadow)
            .inner_margin(Margin::ZERO);
        let mut close = false;
        let out = egui::Modal::new(egui::Id::unique("settings-sheet"))
            .frame(frame)
            .show(ctx, |ui| {
                ui.set_width(w);
                ui.set_height(h);
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                ui.horizontal_top(|ui| {
                    egui::Frame::new()
                        .fill(pal.raised)
                        .corner_radius(CornerRadius {
                            nw: 16,
                            sw: 16,
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
                                ui.label(style::semibold("Settings").size(18.0));
                                ui.add_space(10.0);
                                for s in Section::ALL {
                                    let (_, r) = nav_row(
                                        ui,
                                        s.icon(),
                                        s.name(),
                                        &format!("{} settings", s.name()),
                                        None,
                                        self.settings_section == s,
                                        0.0,
                                    );
                                    if r.clicked() {
                                        self.settings_section = s;
                                    }
                                }
                            });
                        });
                    egui::Frame::new()
                        .inner_margin(Margin::symmetric(20, 14))
                        .show(ui, |ui| {
                            ui.vertical(|ui| {
                                ui.set_width(w - nav_w - 40.0);
                                ui.set_height(h - 28.0);
                                ui.spacing_mut().item_spacing = Vec2::new(8.0, 6.0);
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(self.settings_section.name())
                                            .text_style(egui::TextStyle::Heading),
                                    );
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            if style::icon_button_framed(
                                                ui,
                                                icons::ICON_CLOSE,
                                                "Close settings",
                                            )
                                            .clicked()
                                            {
                                                close = true;
                                            }
                                        },
                                    );
                                });
                                ui.add_space(6.0);
                                // A scrollbar that takes its own room: the content never runs
                                // under it.
                                ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
                                egui::ScrollArea::vertical()
                                    .id_salt(("settings-scroll", self.settings_section as u8))
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        ui.add_space(2.0);
                                        // Breathing room between the content and the bar.
                                        ui.set_max_width(ui.available_width() - 8.0);
                                        match self.settings_section {
                                            Section::Appearance => self.settings_appearance(ui),
                                            Section::SmartSearch => self.settings_smart_search(ui),
                                            Section::Library => self.settings_library(ui),
                                            Section::Storage => self.settings_storage(ui),
                                            Section::About => Self::settings_about(ui),
                                        }
                                        ui.add_space(24.0);
                                    });
                            });
                        });
                });
            });
        let nothing_on_top = !self.dialog_open();
        if close || (out.backdrop_response.clicked() && nothing_on_top) {
            self.settings_open = false;
        }
    }

    fn settings_appearance(&mut self, ui: &mut egui::Ui) {
        let s = self.settings.clone();
        heading(ui, "Colors");
        if let Some(t) = segmented(
            ui,
            &[
                (Theme::System, "Match my computer"),
                (Theme::Light, "Light"),
                (Theme::Dark, "Dark"),
            ],
            s.theme,
        ) {
            self.change_settings(|s| s.theme = t);
        }
        hint(
            ui,
            "“Match my computer” follows your system’s light or dark setting.",
        );
        ui.add_space(14.0);
        heading(ui, "Text size");
        if let Some(t) = segmented(
            ui,
            &[(TextSize::Normal, "Normal"), (TextSize::Large, "Large")],
            s.text_size,
        ) {
            self.change_settings(|s| s.text_size = t);
        }
        hint(ui, "Large makes all text and buttons about 15 % bigger.");
        ui.add_space(14.0);
        heading(ui, "Tips");
        ui.label("Short hints appear now and then to show you what Recollect can do.");
        ui.add_space(4.0);
        if ui.button("Show tips again").clicked() {
            self.change_settings(|s| s.tips_dismissed.clear());
        }
    }

    fn settings_smart_search(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        let installed = self.snapshot.model_installed;
        let current = self.lib.model();
        let status = smart_search_status(&self.status.activity, installed, current);
        let downloading = matches!(self.status.activity, Activity::Downloading { .. });
        style::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let on = installed && !downloading;
                let (icon, color) = if on {
                    (icons::ICON_CHECK_CIRCLE, pal.accent)
                } else {
                    (icons::ICON_AUTO_AWESOME, pal.text_secondary)
                };
                ui.label(style::icon_text(icon, 22.0).color(color));
                ui.label(style::semibold(status).size(15.0));
            });
            if let Activity::Downloading { done, total, .. } = &self.status.activity {
                ui.add_space(4.0);
                ui.add(
                    egui::ProgressBar::new(*done as f32 / (*total).max(1) as f32)
                        .desired_height(6.0)
                        .corner_radius(3.0),
                );
            }
        });
        ui.add_space(8.0);
        ui.label(
            "Smart search finds photos and videos when you describe them, like “birthday \
             cake”. It runs on this computer.",
        );
        heading(ui, "Choose one");
        for m in registry::MODELS {
            let here = self.snapshot.installed_models.contains(&m.id);
            let note = if here { "Already on this computer" } else { "" };
            if choice_card(
                ui,
                m.id == current.id,
                quality_name(m),
                &quality_blurb(m),
                note,
            ) && m.id != current.id
            {
                self.model_confirm = Some(m.id);
            }
            ui.add_space(4.0);
        }
        hint(
            ui,
            "Switching makes Recollect look at your photos again, which can take a while.",
        );
        if !installed && !downloading {
            ui.add_space(8.0);
            let label = format!("Download and turn on ({} MB)", current.size_mb());
            let b = egui::Button::new(RichText::new(&label).color(pal.on_accent)).fill(pal.accent);
            let r = ui.add(b);
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
            if r.clicked() {
                self.turn_on_search();
            }
            hint(
                ui,
                "A one-time download. After that, search works without the internet.",
            );
        }
        ui.add_space(10.0);
        let mut gpu = self.settings.use_gpu;
        if ui
            .checkbox(&mut gpu, "Use the graphics card when available")
            .changed()
        {
            self.change_settings(|s| s.use_gpu = gpu);
        }
        ui.add_space(6.0);
        egui::CollapsingHeader::new("Details")
            .id_salt("settings-search-details")
            .show(ui, |ui| {
                ui.label(format!("Name: {}", current.name));
                ui.label(format!("License: {}", current.license));
                ui.label(format!("Source: huggingface.co/{}", current.repo));
            });
    }

    fn settings_library(&mut self, ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        let busy = self.lib.is_busy();
        heading(ui, "Folders");
        if self.snapshot.folders.is_empty() {
            ui.label("No folders yet. Add the folders where your photos and videos are.");
        }
        let labels = folder_labels(self.snapshot.folders.iter().map(|f| f.path.as_path()));
        let rows: Vec<(i64, String, String)> = self
            .snapshot
            .folders
            .iter()
            .zip(labels)
            .map(|(f, l)| (f.id, l, f.path.display().to_string()))
            .collect();
        for (id, name, path) in rows {
            egui::Frame::new()
                .fill(pal.raised)
                .corner_radius(CornerRadius::same(style::CARD_RADIUS))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(
                            style::icon_text(icons::ICON_FOLDER, 22.0).color(pal.text_secondary),
                        );
                        let stop_label =
                            format!("Stop showing “{}”…", super::albums::short_name(&name, 24));
                        let button_w = ui
                            .painter()
                            .layout_no_wrap(
                                stop_label.clone(),
                                egui::TextStyle::Button.resolve(ui.style()),
                                pal.text,
                            )
                            .size()
                            .x
                            + 40.0;
                        let text_w = (ui.available_width() - button_w - 16.0).max(80.0);
                        ui.vertical(|ui| {
                            ui.set_max_width(text_w);
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.add(egui::Label::new(style::semibold(&name)).truncate());
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&path).small().color(pal.text_secondary),
                                )
                                .truncate(),
                            );
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(&stop_label).clicked() {
                                self.stop_folder = Some(super::sidebar::StopFolder {
                                    id,
                                    name: name.clone(),
                                });
                            }
                        });
                    });
                });
            ui.add_space(2.0);
        }
        ui.add_space(4.0);
        if style::icon_label_button(ui, icons::ICON_CREATE_NEW_FOLDER, "Add a folder…").clicked()
        {
            self.open_add_folder();
        }

        heading(ui, "Your library");
        let stats = self.snapshot.stats.clone();
        let text = counts_text(&stats, self.snapshot.model_installed);
        if text.is_empty() {
            ui.label("No photos or videos yet.");
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.label(text);
                if stats.errors > 0
                    && ui
                        .link(if self.failed_open { "Hide" } else { "Show" })
                        .clicked()
                {
                    self.failed_open = !self.failed_open;
                }
            });
        }
        if stats.errors > 0 && self.failed_open {
            hint(
                ui,
                "These files couldn't be opened. They may be damaged or unsupported.",
            );
            egui::Frame::new()
                .fill(pal.raised)
                .corner_radius(CornerRadius::same(style::CARD_RADIUS))
                .inner_margin(Margin::same(10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.failed_files_list(ui, "settings-failed", 140.0);
                });
        }
        ui.add_space(8.0);
        ui.add_enabled_ui(!busy && !self.snapshot.folders.is_empty(), |ui| {
            if style::icon_label_button(ui, icons::ICON_REFRESH, "Check for new photos").clicked() {
                self.start_indexing();
            }
        });
        hint(
            ui,
            "Recollect also checks when it starts. Use this after adding photos to a folder.",
        );

        ui.add_space(14.0);
        egui::CollapsingHeader::new("Advanced")
            .id_salt("settings-advanced")
            .show(ui, |ui| {
                let s = self.settings.clone();
                let max =
                    std::thread::available_parallelism().map_or(4, std::num::NonZero::get) as u64;
                ui.label("Indexing threads (decoding workers while photos are indexed)");
                if let Some(n) = committed_slider(
                    ui,
                    "Indexing threads",
                    s.decode_threads() as u64,
                    1..=max,
                    1.0,
                    |n| format!("{}", n as u64),
                ) {
                    self.change_settings(|s| {
                        s.index_threads = Some(usize::try_from(n).unwrap_or(usize::MAX));
                    });
                }
                ui.add_space(6.0);
                ui.label(format!(
                    "Embedding model: {} ({})",
                    self.lib.model().name,
                    self.lib.model().id
                ));
                ui.label(
                    RichText::new(format!(
                        "Model files: {}",
                        self.lib.paths().models_dir().display()
                    ))
                    .small(),
                );
            });
    }

    fn settings_storage(&mut self, ui: &mut egui::Ui) {
        heading(ui, "Space for previews");
        ui.label(
            "Recollect keeps small copies of your photos so scrolling stays fast. They are \
             made again whenever they are needed, so you can give them as much or as little \
             room as you like.",
        );
        ui.add_space(6.0);
        let current = self.settings.thumb_cache_mb;
        if let Some(mb) = committed_slider(
            ui,
            "Space for previews",
            current,
            64..=8192,
            64.0,
            fmt_space,
        ) {
            self.change_settings(|s| s.thumb_cache_mb = mb);
            prune_thumbnails(&self.lib);
        }

        ui.add_space(12.0);
        heading(ui, "Where Recollect keeps its data");
        hint(
            ui,
            "Recollect never writes anything into your photo folders.",
        );
        ui.add_space(4.0);
        let paths = self.lib.paths().clone();
        for (what, desc, dir) in [
            (
                "library",
                "Your library and smart search",
                paths.data.clone(),
            ),
            ("previews", "Previews", paths.cache.clone()),
            ("settings", "Settings", paths.config),
        ] {
            let pal = Palette::of(ui.ctx());
            egui::Frame::new()
                .fill(pal.raised)
                .corner_radius(CornerRadius::same(style::CARD_RADIUS))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let text_w = (ui.available_width() - 140.0).max(80.0);
                        ui.vertical(|ui| {
                            ui.set_max_width(text_w);
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(style::semibold(desc));
                            ui.add(
                                egui::Label::new(
                                    RichText::new(dir.display().to_string())
                                        .small()
                                        .color(pal.text_secondary),
                                )
                                .truncate(),
                            )
                            .on_hover_text(dir.display().to_string());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let r = ui.button("Open folder");
                            r.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::Role::Button,
                                    true,
                                    format!("Open folder for {what}"),
                                )
                            });
                            if r.clicked() {
                                let _ = std::fs::create_dir_all(&dir);
                                if let Err(e) = platform::open(&dir) {
                                    self.notify_problem(
                                        "Couldn't open that folder. Open it from your file manager \
                                         instead.",
                                        format!("{e:#}"),
                                        None,
                                    );
                                }
                            }
                        });
                    });
                });
            ui.add_space(2.0);
        }
    }

    fn settings_about(ui: &mut egui::Ui) {
        let pal = Palette::of(ui.ctx());
        ui.horizontal(|ui| {
            ui.label(style::icon_text(icons::ICON_PHOTO_LIBRARY, 34.0).color(pal.accent));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(style::semibold("Recollect").size(18.0));
                ui.label(
                    RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                        .color(pal.text_secondary),
                );
            });
        });
        heading(ui, "Your photos stay private");
        ui.label(
            "Everything Recollect does happens on this computer. Your photos, videos and \
             searches never leave it, and nothing is sent to anyone. The only time Recollect \
             uses the internet is the one-time download for smart search.",
        );
        heading(ui, "Free and open source");
        ui.label(
            "Recollect is free software. You can read how it works, and how to build it, in \
             the project's documentation.",
        );
        ui.add_space(8.0);
        egui::CollapsingHeader::new("Open-source licenses")
            .id_salt("settings-licenses")
            .show(ui, |ui| {
                for line in [
                    "Recollect: MIT or Apache License 2.0",
                    "Inter typeface: SIL Open Font License 1.1",
                    "Material Symbols icons: Apache License 2.0",
                    "ONNX Runtime: MIT License",
                    "FFmpeg: GNU LGPL",
                ] {
                    ui.label(line);
                }
            });
    }

    /// The question before switching smart search: it makes every photo get looked at again.
    pub(super) fn model_confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(id) = self.model_confirm else {
            return;
        };
        let Some(spec) = registry::get(id) else {
            self.model_confirm = None;
            return;
        };
        let here = self.snapshot.installed_models.contains(&spec.id);
        let (mut switch, mut cancel) = (false, false);
        modal_window(ctx, &format!("Switch to {}?", quality_name(spec)), |ui| {
            ui.set_max_width(420.0);
            ui.label(
                "Recollect will look at all your photos again so search works with this \
                 choice. That can take a while, and you can keep using Recollect meanwhile.",
            );
            if !here {
                ui.add_space(4.0);
                ui.label(format!(
                    "It needs a one-time download of {} MB.",
                    spec.size_mb()
                ));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                switch = ui.button("Switch").clicked();
                cancel = ui.button("Cancel").clicked();
            });
        });
        if switch {
            self.model_confirm = None;
            self.switch_model(spec);
        } else if cancel {
            self.model_confirm = None;
        }
    }

    /// Makes `spec` the smart-search choice (waits if a long task is running).
    fn switch_model(&mut self, spec: &'static ModelSpec) {
        if self.lib.is_busy() {
            self.notify(
                "Recollect is busy right now. Wait for it to finish, or press Stop in the left \
                 panel, then try again.",
            );
            return;
        }
        if let Err(e) = self.lib.set_model(spec.id) {
            self.notify_problem(
                "Couldn't change smart search. Please try again in a moment.",
                format!("{e:#}"),
                None,
            );
        } else {
            // Old results and queries belong to the previous choice.
            self.set_search_prompt_dismissed(false);
            self.end_viewer();
            self.clear_search();
            self.refresh_snapshot(true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_is_plain() {
        let best = &registry::SIGLIP_B16;
        let fast = &registry::CLIP_B32;
        assert_eq!(
            smart_search_status(&Activity::Idle, true, best),
            "On — best quality"
        );
        assert_eq!(
            smart_search_status(&Activity::Idle, true, fast),
            "On — faster"
        );
        assert_eq!(smart_search_status(&Activity::Idle, false, best), "Off");
        let dl = Activity::Downloading {
            model: "m".into(),
            done: 120_000_000,
            total: 485_000_000,
        };
        assert_eq!(
            smart_search_status(&dl, false, best),
            "Downloading… 120 of 485 MB"
        );
        assert_eq!(
            smart_search_status(&Activity::LoadingModel, true, best),
            "Getting your photos ready…"
        );
    }

    #[test]
    fn choices_name_their_size() {
        assert!(quality_blurb(&registry::SIGLIP_B16).ends_with("486 MB."));
        assert!(quality_blurb(&registry::CLIP_B32).ends_with("608 MB."));
    }

    #[test]
    fn counts_read_in_one_line() {
        let stats = Stats {
            images: 1204,
            videos: 37,
            indexed: 1241,
            errors: 3,
            ..Default::default()
        };
        assert_eq!(
            counts_text(&stats, true),
            "1,204 photos · 37 videos · 1,241 ready for search · 3 couldn't be opened"
        );
        assert_eq!(
            counts_text(&stats, false),
            "1,204 photos · 37 videos · 3 couldn't be opened"
        );
        assert_eq!(counts_text(&Stats::default(), true), "");
    }

    #[test]
    fn space_is_written_in_mb_and_gb() {
        assert_eq!(fmt_space(512.0), "512 MB");
        assert_eq!(fmt_space(1024.0), "1 GB");
        assert_eq!(fmt_space(1536.0), "1.5 GB");
        assert_eq!(parse_space("2 GB"), Some(2048.0));
        assert_eq!(parse_space("300"), Some(300.0));
        assert_eq!(parse_space("300 MB"), Some(300.0));
        assert_eq!(parse_space("lots"), None);
    }
}
