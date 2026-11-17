//! The search results screen: heading, filters, the preparing note, the empty state and what
//! the result tiles offer.

use super::*;
use crate::library::{Activity, IndexProgress};

fn results_harness(d: &std::path::Path) -> (Rc<RefCell<App>>, Harness<'static>) {
    // Without the smart search files, the searches these tests start fail cleanly.
    let app = Rc::new(RefCell::new(grid_app_with(d, false)));
    let h = album_harness(&app);
    (app, h)
}

#[test]
fn the_heading_counts_what_is_shown_after_the_filters() {
    let d = tempfile::tempdir().unwrap();
    let (app, mut h) = results_harness(d.path());
    h.get_by_label("3 results");
    // The words searched for stay beside the count, quieter.
    h.get_by_label("for “a dragon”");
    // A narrower answer (as the search returns after a filter) changes the count.
    let one = vec![app.borrow().res.hits[0].clone()];
    app.borrow_mut().res.hits = Arc::new(one);
    h.run_steps(2);
    h.get_by_label("1 result");
    // Choosing a filter searches again with it.
    h.get_by_label("Videos only").click();
    h.run_steps(3);
    assert_eq!(app.borrow().kind, Some(MediaKind::Video));
    assert!(app.borrow().res.awaiting.is_some() || app.borrow().res.error.is_some());
}

#[test]
fn large_counts_have_a_thousands_separator() {
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app(d.path());
    let h0 = app.res.hits[0].clone();
    app.res.hits = Arc::new((0..1200).map(|_| h0.clone()).collect());
    let app = Rc::new(RefCell::new(app));
    let h = album_harness(&app);
    h.get_by_label("1,200 results");
}

#[test]
fn the_preparing_note_shows_only_while_photos_are_being_prepared() {
    let d = tempfile::tempdir().unwrap();
    let (app, mut h) = results_harness(d.path());
    let lib = Arc::clone(&app.borrow().lib);
    assert!(h.query_by_label_contains("Still getting").is_none());
    lib.update_status(|s| {
        s.activity = Activity::Indexing(IndexProgress {
            done: 200,
            total: 1434,
            ..Default::default()
        });
    });
    h.run_steps(2);
    h.get_by_label("Still getting 1,234 photos ready \u{2014} results may grow");
    lib.update_status(|s| s.activity = Activity::Idle);
    h.run_steps(2);
    assert!(h.query_by_label_contains("Still getting").is_none());
}

#[test]
fn no_results_with_a_filter_offers_clear_filters() {
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app_with(d.path(), false);
    app.res.hits = Arc::default();
    app.kind = Some(MediaKind::Video);
    app.date = crate::ui::when::DateFilter {
        when: crate::ui::when::When::LastYear,
        from: 0,
        to: 0,
    };
    app.sort = crate::search::SearchSort::Oldest;
    let app = Rc::new(RefCell::new(app));
    let mut h = album_harness(&app);
    h.get_by_label("0 results");
    h.get_by_label("Clear filters").click();
    h.run_steps(3);
    let a = app.borrow();
    assert_eq!(a.kind, None);
    assert_eq!(a.date, crate::ui::when::DateFilter::default());
    assert_eq!(a.sort, crate::search::SearchSort::Relevance);
    assert!(a.res.current.is_some(), "the words stay");
}

#[test]
fn no_results_without_a_filter_hints_at_other_words() {
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app(d.path());
    app.res.hits = Arc::default();
    let app = Rc::new(RefCell::new(app));
    let h = album_harness(&app);
    h.get_by_label(
        "No photos match \u{201c}a dragon\u{201d}. Try describing the moment in other words.",
    );
    assert!(h.query_by_label("Clear filters").is_none());
}

#[test]
fn find_similar_on_a_hovered_tile_starts_a_similar_search() {
    let d = tempfile::tempdir().unwrap();
    let (app, mut h) = results_harness(d.path());
    app.borrow_mut().query = "a dragon".into();
    h.get_by_label("Result b.jpg").hover();
    h.run_steps(2);
    h.get_by_label("Find similar to b.jpg").click();
    h.run_steps(3);
    let a = app.borrow();
    assert_eq!(
        a.res.current,
        Some(crate::ui::Query::Similar {
            item_id: 1,
            label: "b.jpg".into()
        })
    );
    assert!(a.query.is_empty(), "nothing was typed");
    assert!(a.viewer.is_none(), "the click did not open the photo");
}

#[test]
fn a_video_result_plays_at_its_moment_from_the_pill() {
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app(d.path());
    Arc::make_mut(&mut app.res.hits)[0] = hit("/nowhere/clip.mp4", MediaKind::Video, Some(42.0));
    let app = Rc::new(RefCell::new(app));
    let mut h = album_harness(&app);
    // Images have no such button.
    assert_eq!(h.query_all_by_label_contains("Play at").count(), 1);
    h.get_by_label("Play at 0:42").click();
    h.run_steps(3);
    let a = app.borrow();
    assert_eq!(a.res.played, [("/nowhere/clip.mp4".into(), 42.0)]);
    assert!(a.viewer.is_none(), "the pill does not open the viewer");
}

#[test]
fn results_select_with_shift_and_show_the_selection_bar() {
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app(d.path());
    Arc::make_mut(&mut app.res.hits)[1].file_id = 2;
    let app = Rc::new(RefCell::new(app));
    let mut h = album_harness(&app);
    h.get_by_label("Select a.jpg").click();
    h.run_steps(3);
    click_with(&mut h, "Result c.jpg", egui::Modifiers::SHIFT);
    h.get_by_label("3 selected");
    h.get_by_label("Add to album…");
    h.get_by_label("Favorite");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app.borrow().res.grid.sel.is_empty());
    assert!(h.query_by_label("3 selected").is_none());
}

#[test]
fn while_selecting_the_find_similar_corner_selects_the_tile_instead() {
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app(d.path());
    for (i, h) in Arc::make_mut(&mut app.res.hits).iter_mut().enumerate() {
        h.file_id = i as i64 + 1;
    }
    let before = app.res.current.clone();
    let app = Rc::new(RefCell::new(app));
    let mut h = album_harness(&app);
    h.get_by_label("Select a.jpg").click();
    h.run_steps(3);
    let tile = h.get_by_label("Result b.jpg").rect();
    let corner = egui::pos2(tile.max.x - 12.0, tile.min.y + 12.0);
    h.hover_at(corner);
    h.run_steps(2);
    assert!(h.query_by_label_contains("Find similar to").is_none());
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton {
            pos: corner,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(2);
    }
    let a = app.borrow();
    assert_eq!(a.res.current, before, "no similar search was started");
    assert_eq!(a.res.grid.sel.len(), 2, "the tile was selected");
}
