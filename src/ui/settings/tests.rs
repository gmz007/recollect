use super::library::counts_text;
use super::smart_search::smart_search_status;
use super::widgets::fmt_space;
use super::widgets::parse_space;
use super::*;
use crate::library::Activity;
use crate::model::registry;
use crate::store::db::Stats;

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

/// A row with a 150 point control, in a `width` wide window: the title's and the control's
/// rectangles.
fn row_rects(width: f32) -> (egui::Rect, egui::Rect) {
    use egui_kittest::kittest::Queryable as _;
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(width, 300.0))
        .build_ui(|ui| {
            // The app's fonts are installed after the harness is built; they load a few
            // passes later.
            if ui.ctx().cumulative_pass_nr() < 3 {
                return;
            }
            super::rows::group(ui, |ui, rows| {
                rows.row(
                    ui,
                    "Row title",
                    Some("A longer line of explanation"),
                    150.0,
                    |ui| {
                        let _ = ui.button("Control");
                    },
                );
            });
        });
    crate::ui::style::apply(
        &h.ctx,
        crate::config::Theme::default(),
        crate::config::TextSize::default(),
    );
    h.run();
    (
        h.get_by_label("Row title").rect(),
        h.get_by_label("Control").rect(),
    )
}

#[test]
fn a_row_puts_its_control_under_the_text_when_both_do_not_fit() {
    let (title, control) = row_rects(300.0);
    assert!(control.min.y >= title.max.y, "{title:?} vs {control:?}");
    let (title, control) = row_rects(700.0);
    assert!(control.min.x >= title.max.x, "side by side when wide");
    assert!(control.min.y < title.max.y);
}
