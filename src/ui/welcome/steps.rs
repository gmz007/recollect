//! What each first-run screen says and asks: its header, headline, text and controls.

use super::{MARK_SIZE, Step, Welcome};
use crate::model::registry;
use crate::ui::App;
use crate::ui::folders::{BAD_FOLDER_PATH, typed_folder};
use crate::ui::settings::{choice_card, quality_blurb, quality_name};
use crate::ui::style::{self, Palette, icons};
use egui::{RichText, Stroke, Ui};
use egui_material_icons::MaterialIcon;

const HEADLINE_SIZE: f32 = 26.0;
const BODY_SIZE: f32 = 14.0;
const ROW_PADDING: i8 = 8;

/// The app mark and name on the left, "1 of 4" on the right.
pub(super) fn welcome_header(ui: &mut Ui, step: Step) {
    let pal = Palette::of(ui.ctx());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        style::app_mark(ui, MARK_SIZE, pal.bg);
        ui.label(style::semibold("Recollect").size(BODY_SIZE));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let count = format!("{} of {}", step.number() + 1, Step::COUNT);
            ui.label(RichText::new(count).color(pal.muted));
        });
    });
}

/// A step's headline and the sentence under it.
fn headline(ui: &mut Ui, title: &str, text: &str) {
    let pal = Palette::of(ui.ctx());
    ui.label(style::semibold(title).size(HEADLINE_SIZE));
    ui.add_space(8.0);
    ui.add(egui::Label::new(
        RichText::new(text).size(BODY_SIZE).color(pal.text2),
    ));
    ui.add_space(20.0);
}

/// A bordered block of rows, divided by hairlines.
fn card(ui: &mut Ui, add_rows: impl FnOnce(&mut Ui)) {
    let pal = Palette::of(ui.ctx());
    egui::Frame::new()
        .stroke(Stroke::new(1.0, pal.line))
        .corner_radius(style::CARD_RADIUS)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add_rows(ui);
        });
}

/// A hairline across the card, between two rows.
fn divider(ui: &mut Ui) {
    let pal = Palette::of(ui.ctx());
    let (r, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, pal.line);
}

/// A row of a card with the standard padding.
fn row(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(14, ROW_PADDING))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// One privacy or benefit line: an icon, a bold lead and the rest.
fn benefit(ui: &mut Ui, icon: MaterialIcon, lead: &str, rest: &str) {
    let pal = Palette::of(ui.ctx());
    ui.horizontal_top(|ui| {
        ui.label(style::icon_text(icon, 18.0).color(pal.text2));
        let font = egui::FontId::new(BODY_SIZE, egui::TextStyle::Body.resolve(ui.style()).family);
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = ui.available_width();
        job.append(
            lead,
            0.0,
            egui::TextFormat {
                font_id: egui::FontId::new(BODY_SIZE, style::semibold_family()),
                color: pal.text,
                ..Default::default()
            },
        );
        job.append(
            &format!(" {rest}"),
            0.0,
            egui::TextFormat {
                font_id: font,
                color: pal.text2,
                ..Default::default()
            },
        );
        ui.add(egui::Label::new(job).wrap());
    });
    ui.add_space(12.0);
}

/// A picture of the search field with an example, so the first screen shows what it is for.
fn example_search(ui: &mut Ui) {
    const HEIGHT: f32 = 38.0;
    let pal = Palette::of(ui.ctx());
    let (r, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), HEIGHT),
        egui::Sense::hover(),
    );
    let p = ui.painter();
    p.rect_filled(r, style::FIELD_RADIUS, pal.field);
    let icon_at = egui::pos2(r.min.x + 20.0, r.center().y);
    p.text(
        icon_at,
        egui::Align2::CENTER_CENTER,
        icons::ICON_SEARCH.codepoint,
        style::icon_font_id(icons::ICON_SEARCH, 16.0),
        pal.muted,
    );
    p.text(
        egui::pos2(r.min.x + 36.0, r.center().y),
        egui::Align2::LEFT_CENTER,
        "grandma's birthday cake",
        egui::FontId::proportional(BODY_SIZE),
        pal.text,
    );
    ui.add_space(20.0);
}

/// The first screen: what Recollect is for and the privacy promise.
pub(super) fn welcome_intro(ui: &mut Ui) {
    headline(
        ui,
        "Find any photo by describing it",
        "Type what you remember and Recollect finds the photo, or the moment in a video.",
    );
    example_search(ui);
    benefit(
        ui,
        icons::ICON_LOCK,
        "Private.",
        "Everything stays on this computer. No account needed.",
    );
    benefit(
        ui,
        icons::ICON_SHIELD,
        "Safe.",
        "Your photos are only looked at, never moved, changed or deleted.",
    );
    benefit(
        ui,
        icons::ICON_IMAGE,
        "Organized.",
        "Browse by date, keep favorites and make albums.",
    );
}

/// What the last screen says will happen after Start.
fn done_text(download: bool) -> String {
    let mut t = String::from(
        "After you press Start, Recollect gets your photos ready. Progress shows at the \
         bottom left; you can browse right away.",
    );
    t.push_str(if download {
        " Search works once the download finishes."
    } else {
        " You can turn on smart search any time from Settings."
    });
    t
}

/// The last screen: what happens after Start, and what was chosen.
pub(super) fn welcome_done(ui: &mut Ui, w: &Welcome) {
    let pal = Palette::of(ui.ctx());
    headline(ui, "You're all set", &done_text(w.download));
    let n = w.folders().len();
    let folders = format!("{n} {}", if n == 1 { "folder" } else { "folders" });
    let search = if w.download {
        "Downloading, then on"
    } else {
        "Off for now"
    };
    card(ui, |ui| {
        for (i, (name, value)) in [("Folders", folders.as_str()), ("Smart search", search)]
            .into_iter()
            .enumerate()
        {
            if i > 0 {
                divider(ui);
            }
            row(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).color(pal.text2));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(style::medium(value));
                    });
                });
            });
        }
    });
}

/// A friendly second line for a standard folder.
fn suggestion_note(name: &str) -> String {
    match name {
        "Desktop" => "Files on your desktop".to_string(),
        "Downloads" => "Things you have downloaded".to_string(),
        other => format!("Your {other} folder"),
    }
}

/// The folders the user picked or typed so far, each with a button to take it back.
fn welcome_added_folders(ui: &mut Ui, w: &mut Welcome) {
    if w.added.is_empty() {
        return;
    }
    let pal = Palette::of(ui.ctx());
    ui.add_space(10.0);
    ui.label(style::semibold("Folders you added").size(BODY_SIZE));
    let mut remove = None;
    for (i, p) in w.added.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.label(style::icon_text(icons::ICON_FOLDER, 18.0).color(pal.text2));
            ui.vertical(|ui| {
                let name = p.file_name().map_or_else(
                    || p.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                );
                ui.add(egui::Label::new(name).truncate());
                let path = RichText::new(p.display().to_string())
                    .small()
                    .color(pal.muted);
                ui.add(egui::Label::new(path).truncate());
            });
            if style::icon_button(ui, icons::ICON_CLOSE, &format!("Remove {}", p.display()))
                .clicked()
            {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        w.added.remove(i);
    }
}

/// The smart-search screen: the two choices with their sizes, and the privacy note.
pub(super) fn welcome_search(ui: &mut Ui, w: &mut Welcome) {
    let pal = Palette::of(ui.ctx());
    headline(
        ui,
        "Turn on smart search",
        "Describe what you remember, like “kids on the beach”, and Recollect finds it. \
         You can browse your photos right away either way.",
    );
    let recommended = w.recommended;
    for m in registry::MODELS {
        let note = if m.id == recommended {
            "Recommended for this computer"
        } else {
            ""
        };
        if choice_card(
            ui,
            m.id == w.choice,
            quality_name(m),
            &quality_blurb(m),
            note,
        ) {
            w.choice = m.id;
        }
        ui.add_space(6.0);
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.label(style::icon_text(icons::ICON_LOCK, 18.0).color(pal.text2));
        ui.label(
            RichText::new(
                "It downloads once. After that it works without the internet, and your \
                 photos never leave this computer.",
            )
            .color(pal.text2),
        );
    });
}

impl App {
    pub(super) fn welcome_folders(&mut self, ui: &mut Ui, w: &mut Welcome) {
        let pal = Palette::of(ui.ctx());
        headline(
            ui,
            "Where are your photos?",
            "Choose the folders to look in. You can change these any time.",
        );
        card(ui, |ui| welcome_folder_rows(ui, w));
        if !w
            .suggestions
            .iter()
            .any(|s| s.name == "Pictures" || s.name == "Videos")
        {
            ui.add_space(8.0);
            ui.label("We didn't find a Pictures folder. Choose where your photos are.");
        }
        welcome_added_folders(ui, w);
        ui.add_space(8.0);
        self.welcome_typed_path(ui, w);
        if w.folders().is_empty() {
            ui.add_space(10.0);
            ui.label(RichText::new("Pick at least one folder to continue.").color(pal.muted));
        }
    }

    /// A text field for computers where the folder picker is missing or awkward.
    fn welcome_typed_path(&mut self, ui: &mut Ui, w: &mut Welcome) {
        let pal = Palette::of(ui.ctx());
        egui::CollapsingHeader::new("More options")
            .id_salt("welcome-more-options")
            .show(ui, |ui| {
                ui.label(
                    RichText::new("Or type a folder path")
                        .small()
                        .color(pal.text2),
                );
                let mut add_typed = false;
                ui.horizontal(|ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut w.typed)
                            .hint_text("Type or paste the folder's location")
                            .desired_width(ui.available_width() - 130.0),
                    );
                    r.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::TextInput, true, "Folder path")
                    });
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let can = !w.typed.trim().is_empty();
                    add_typed = (ui
                        .add_enabled(can, egui::Button::new("Add this folder"))
                        .clicked()
                        || enter)
                        && can;
                });
                if add_typed {
                    if let Some(p) = typed_folder(&w.typed) {
                        if !w.added.contains(&p) {
                            w.added.push(p);
                        }
                        w.typed.clear();
                    } else {
                        self.notify_error(BAD_FOLDER_PATH);
                    }
                }
            });
    }
}

/// The folder card's rows: a checkbox per suggestion, then "Add a folder…".
fn welcome_folder_rows(ui: &mut Ui, w: &mut Welcome) {
    let pal = Palette::of(ui.ctx());
    for s in &mut w.suggestions {
        row(ui, |ui| {
            ui.horizontal(|ui| {
                let r = ui
                    .checkbox(&mut s.checked, style::medium(&s.name))
                    .on_hover_text(s.path.display().to_string());
                r.widget_info(|| {
                    egui::WidgetInfo::selected(egui::Role::CheckBox, true, s.checked, &s.name)
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(suggestion_note(&s.name)).color(pal.muted));
                });
            });
        });
        divider(ui);
    }
    row(ui, |ui| {
        if add_folder_button(ui).clicked()
            && let Some(dir) = rfd::FileDialog::new()
                .set_title("Choose a folder with photos and videos")
                .pick_folder()
            && !w.added.contains(&dir)
        {
            w.added.push(dir);
        }
    });
}

/// The accent-coloured "Add a folder…" line at the end of the folder list.
fn add_folder_button(ui: &mut Ui) -> egui::Response {
    let pal = Palette::of(ui.ctx());
    let label = "Add a folder…";
    let job = style::icon_label_job(
        ui,
        icons::ICON_ADD,
        18.0,
        pal.accent,
        Some((label, pal.accent)),
        8.0,
    );
    let r = ui.add(egui::Button::new(job).frame(false));
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    r.on_hover_cursor(egui::CursorIcon::PointingHand)
}
