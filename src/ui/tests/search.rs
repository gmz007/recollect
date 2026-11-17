//! Search results, search options and the search field.

use super::*;
use std::time::{Duration, Instant};

#[test]
fn results_show_a_plain_header_and_square_tiles() {
    use crate::media::MediaKind;
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    fake_install(&lib);
    let mut app = App::new(&egui::Context::default(), lib);
    app.frame_stats = None;
    // No real folder: adding one would start a background scan that races the test.
    app.snapshot.folders = vec![crate::store::db::Folder {
        id: 1,
        path: "/nowhere".into(),
    }];
    app.res.hits = Arc::new(vec![
        hit("/nowhere/movie.mp4", MediaKind::Video, Some(42.0)),
        hit("/nowhere/photo.jpg", MediaKind::Image, None),
    ]);
    app.res.current = Some(crate::ui::Query::Text("a dragon".into()));
    app.res.label = "a dragon".into();
    let mut h = Harness::builder()
        .with_size(egui::vec2(1200.0, 800.0))
        .with_max_steps(super::MAX_STEPS)
        .build_ui(move |ui| app.show(ui));
    h.run();
    h.get_by_label("2 results");
    assert!(h.query_by_label_contains("Results for").is_none());
    h.get_by_label("Result movie.mp4");
    h.get_by_label("Result photo.jpg");
    h.get_by_label("Clear search");
    assert!(h.query_by_label_contains("score").is_none());
    assert!(h.query_by_label_contains(" ms").is_none());
}

#[test]
fn arrow_keys_move_the_selection() {
    let d = tempfile::tempdir().unwrap();
    let mut h = grid_harness(grid_app(d.path()));
    assert!(!is_selected(&h, "Result c.jpg"));
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!(is_selected(&h, "Result a.jpg"));
    h.key_press(egui::Key::ArrowRight);
    h.run();
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!(is_selected(&h, "Result c.jpg"));
    h.key_press(egui::Key::ArrowLeft);
    h.run();
    assert!(is_selected(&h, "Result b.jpg"));
    assert!(!is_selected(&h, "Result c.jpg"));
}

#[test]
fn typing_in_search_blocks_arrows_until_focus_leaves() {
    let d = tempfile::tempdir().unwrap();
    let mut h = grid_harness(grid_app(d.path()));
    h.get_all_by_role(egui::accesskit::Role::TextInput)
        .next()
        .unwrap()
        .click();
    h.run();
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!(!is_selected(&h, "Result a.jpg"));
    h.key_press(egui::Key::Escape);
    h.run();
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!(is_selected(&h, "Result a.jpg"));
}

#[test]
fn modified_arrows_do_not_move_the_selection() {
    let d = tempfile::tempdir().unwrap();
    let mut h = grid_harness(grid_app(d.path()));
    h.key_press_modifiers(egui::Modifiers::ALT, egui::Key::ArrowRight);
    h.run();
    assert!(!is_selected(&h, "Result a.jpg"));
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!(is_selected(&h, "Result a.jpg"));
}

#[test]
fn arrows_are_ignored_while_settings_is_open() {
    let d = tempfile::tempdir().unwrap();
    let mut h = grid_harness(grid_app(d.path()));
    h.get_by_label("Settings").click();
    h.run();
    h.get_by_label("Appearance");
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!(!is_selected(&h, "Result a.jpg"));
}

#[test]
fn clicking_a_tile_keeps_arrows_working() {
    let d = tempfile::tempdir().unwrap();
    let mut h = grid_harness(grid_app(d.path()));
    h.get_by_label("Result a.jpg")
        .click_modifiers(egui::Modifiers::COMMAND);
    h.run();
    h.key_press(egui::Key::ArrowRight);
    h.run();
    assert!(is_selected(&h, "Result b.jpg"));
    assert!(!is_selected(&h, "Result a.jpg"));
}

#[test]
fn viewer_from_a_search_hit_offers_the_moments_of_a_video() {
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app(d.path());
    let results = Arc::make_mut(&mut app.res.hits);
    results[0] = hit("/nowhere/clip.mp4", MediaKind::Video, Some(42.0));
    for (i, h) in results.iter_mut().enumerate() {
        h.file_id = i as i64 + 1;
    }
    app.res.grid.selected = Some(0);
    app.res.selected_t = Some(42.0);
    let app = Rc::new(RefCell::new(app));
    let mut h = album_harness(&app);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    h.get_by_label("clip.mp4");
    h.get_by_label("Play at 0:42");
    h.get_by_label("Matching moments");
    h.get_by_label("0:42 – 0:44");
    h.get_by_label("1:40 – 1:44").click();
    h.run_steps(3);
    h.get_by_label("Play at 1:40");
    // Next goes to the following hit (an image).
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(3);
    h.get_by_label("b.jpg");
    assert!(h.query_by_label("Matching moments").is_none());
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert_eq!(app.borrow().res.grid.selected, Some(1));
}

#[test]
fn search_shows_the_when_and_sort_chips_and_the_top_bar_filters_the_type() {
    let d = tempfile::tempdir().unwrap();
    let app = Rc::new(RefCell::new(grid_app_with(d.path(), false)));
    let mut h = album_harness(&app);
    h.get_by_label("When: Any time");
    h.get_by_label("Sort: Best match");
    // The type filter exists once: the top bar's All / Photos / Videos.
    assert!(h.query_by_label_contains("Type:").is_none());
    h.get_by_label("Everything");
    h.get_by_label("Photos only");
    assert_eq!(app.borrow().date.when, crate::ui::when::When::Any);
    h.get_by_label("When: Any time").click();
    h.run_steps(3);
    h.get_by_label("Last year").click();
    h.run_steps(3);
    assert_eq!(app.borrow().date.when, crate::ui::when::When::LastYear);
    h.get_by_label("When: Last year");
    h.get_by_label("Videos only").click();
    h.run_steps(3);
    assert_eq!(app.borrow().kind, Some(MediaKind::Video));
    assert!(h.query_by_label_contains("Type:").is_none());
    h.get_by_label("Sort: Best match").click();
    h.run_steps(3);
    h.get_by_label("Oldest").click();
    h.run_steps(3);
    assert_eq!(app.borrow().sort, crate::search::SearchSort::Oldest);
    h.get_by_label("Sort: Oldest");
}

#[test]
fn browsing_has_no_when_and_sort_selectors() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let h = album_harness(&app);
    assert!(h.query_by_label_contains("When:").is_none());
}

#[test]
fn years_for_the_when_selector_come_from_the_whole_library() {
    let d = tempfile::tempdir().unwrap();
    let (_lib, app) = favorites_app(d.path());
    let mut h = album_harness(&app);
    // 1_500_000_000 is in 2017; the list is read in the background.
    settle(&mut h, || app.borrow().known_years == [2017]);
}

#[test]
fn cancel_button_and_search_results_multi_select() {
    use crate::media::MediaKind;
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app(d.path());
    app.res.hits = Arc::new(
        (1..=4)
            .map(|i| {
                let mut h = hit(&format!("/nowhere/r{i}.jpg"), MediaKind::Image, None);
                h.file_id = i;
                h
            })
            .collect(),
    );
    let app = Rc::new(RefCell::new(app));
    let mut h = album_harness(&app);
    click_with(&mut h, "Result r1.jpg", egui::Modifiers::COMMAND);
    click_with(&mut h, "Result r3.jpg", egui::Modifiers::SHIFT);
    assert_eq!(tl_selection(&app), [1, 2, 3]);
    h.get_by_label("3 selected");
    h.get_by_label("Add to album…");
    h.get_by_label("Favorite");
    assert!(h.query_by_label("Remove from album").is_none());
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(tl_selection(&app).is_empty());
}

#[test]
fn search_selection_counts_files_and_toggles_per_file() {
    use crate::media::MediaKind;
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app(d.path());
    // r1 and r3 are two hits of file 1.
    app.res.hits = Arc::new(
        [1, 2, 1, 3]
            .iter()
            .enumerate()
            .map(|(k, &f)| {
                let mut h = hit(&format!("/nowhere/r{}.jpg", k + 1), MediaKind::Image, None);
                h.file_id = f;
                h
            })
            .collect(),
    );
    let app = Rc::new(RefCell::new(app));
    let mut h = album_harness(&app);
    click_with(&mut h, "Result r1.jpg", egui::Modifiers::COMMAND);
    // A range over the visible hits r1..r3 covers two files.
    click_with(&mut h, "Result r3.jpg", egui::Modifiers::SHIFT);
    assert_eq!(tl_selection(&app), [1, 2]);
    h.get_by_label("2 selected");
    click_with(&mut h, "Result r4.jpg", egui::Modifiers::COMMAND);
    assert_eq!(tl_selection(&app), [1, 2, 3]);
    h.get_by_label("3 selected");
    // Ctrl+click on the other hit of selected file 1 deselects that file, not everything.
    click_with(&mut h, "Result r3.jpg", egui::Modifiers::COMMAND);
    assert_eq!(tl_selection(&app), [2, 3]);
    h.get_by_label("2 selected");
    assert!(app.borrow().res.grid.sel.contains(3));
}

#[test]
fn results_header_titles_searches_and_similar_photos() {
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app(d.path());
    app.res.current = Some(crate::ui::Query::Similar {
        item_id: 1,
        label: "beach.jpg".into(),
    });
    app.res.label = "similar to beach.jpg".into();
    let app = Rc::new(RefCell::new(app));
    let h = album_harness(&app);
    h.get_by_label("Similar photos");
    assert!(h.query_by_label_contains("results").is_none());
    h.get_by_label("Clear search");
    assert!(h.query_by_label_contains("score").is_none());
}

#[test]
fn starting_a_search_leaves_the_timelines_selection_mode() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let mut h = album_harness(&app);
    h.get_by_label("Select img2.jpg").click();
    h.run_steps(3);
    h.get_by_label("1 selected");
    assert!(app.borrow().tl.grid.picking);
    app.borrow_mut()
        .submit(crate::ui::Query::Text("a dragon".into()));
    h.run_steps(3);
    let a = app.borrow();
    assert!(!a.tl.grid.picking);
    assert!(a.tl.grid.sel.is_empty() && a.tl.grid.selected.is_none());
}

#[test]
fn each_grid_has_its_own_selection_mode() {
    let d = tempfile::tempdir().unwrap();
    let mut app = grid_app_with(d.path(), false);
    // The timeline was in selection mode before the search started.
    app.tl.grid.picking = true;
    let app = Rc::new(RefCell::new(app));
    let mut h = album_harness(&app);
    // A plain click in the results opens the photo.
    h.get_by_label("Result a.jpg").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_some(), "a plain click opens");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none());
    // In the results' own selection mode a plain click selects.
    h.get_by_label("Select b.jpg").click();
    h.run_steps(3);
    assert!(app.borrow().res.grid.picking);
    h.get_by_label("Result c.jpg").click();
    h.run_steps(3);
    assert!(app.borrow().viewer.is_none(), "now clicks select");
    assert_eq!(app.borrow().res.grid.sel.len(), 2);
}

#[test]
fn the_results_heading_follows_the_current_query_kind() {
    let d = tempfile::tempdir().unwrap();
    let app = Rc::new(RefCell::new(grid_app(d.path())));
    let mut h = album_harness(&app);
    h.get_by_label("3 results");
    app.borrow_mut().res.current = Some(crate::ui::Query::Similar {
        item_id: 1,
        label: "beach.jpg".into(),
    });
    h.run_steps(2);
    h.get_by_label("Similar photos");
}

#[test]
fn the_empty_search_field_offers_examples_and_remembers_searches() {
    let d = tempfile::tempdir().unwrap();
    let app = searchable_app(d.path());
    let lib = Arc::clone(&app.borrow().lib);
    let mut h = album_harness(&app);
    assert!(
        h.query_by_label("kids on the beach").is_none(),
        "closed until focused"
    );
    h.get_by_label("Search your photos").click();
    h.run_steps(3);
    for example in crate::ui::guide::EXAMPLE_SEARCHES {
        h.get_by_label(example);
    }
    h.get_by_label("Try");
    assert!(h.query_by_label("Recent").is_none(), "nothing yet");
    // A chip runs that search.
    h.get_by_label("snow").click();
    h.run_steps(3);
    assert_eq!(app.borrow().query, "snow");
    settle(&mut h, || app.borrow().res.error.is_some());
    assert_eq!(
        app.borrow().res.current,
        Some(crate::ui::Query::Text("snow".into()))
    );
    assert_eq!(lib.settings().recent_searches, ["snow"]);
    // Typing and pressing Enter remembers the words too; newest first, no repeats.
    app.borrow_mut().clear_search();
    h.run_steps(2);
    h.get_by_label("Search your photos").click();
    h.run_steps(2);
    h.get_by_label("Search your photos")
        .type_text("sunset boat");
    h.run_steps(2);
    assert_eq!(app.borrow().query, "sunset boat");
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(lib.settings().recent_searches, ["sunset boat", "snow"]);
    app.borrow_mut().remember_search("SNOW");
    assert_eq!(lib.settings().recent_searches, ["SNOW", "sunset boat"]);
    // The most recent eight are kept.
    for i in 0..10 {
        app.borrow_mut().remember_search(&format!("word {i}"));
    }
    assert_eq!(
        lib.settings().recent_searches.len(),
        crate::ui::guide::MAX_RECENT
    );
    // They show under the field, and can be cleared.
    app.borrow_mut().clear_search();
    h.run_steps(2);
    h.get_by_label("Search your photos").click();
    h.run_steps(3);
    h.get_by_label("Recent");
    h.get_by_label("Recent search: word 9");
    h.get_by_label("Clear recent searches").click();
    h.run_steps(3);
    assert!(lib.settings().recent_searches.is_empty());
    assert!(h.query_by_label("Clear recent searches").is_none());
}

#[test]
fn search_help_stays_closed_when_smart_search_is_off_or_text_was_typed() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let mut h = album_harness(&app);
    h.get_by_label(crate::ui::chrome::SEARCH_OFF_LABEL).click();
    h.run_steps(3);
    assert!(h.query_by_label("kids on the beach").is_none());
    let d = tempfile::tempdir().unwrap();
    let app = searchable_app(d.path());
    let mut h = album_harness(&app);
    h.get_by_label("Search your photos").click();
    h.run_steps(2);
    h.get_by_label("Search your photos").type_text("dog");
    h.run_steps(3);
    assert!(h.query_by_label("kids on the beach").is_none());
}

#[test]
fn a_search_error_is_not_shown_twice() {
    let d = tempfile::tempdir().unwrap();
    let app = searchable_app(d.path());
    app.borrow_mut().res.current = Some(crate::ui::Query::Text("snow".into()));
    app.borrow_mut().res.label = "snow".into();
    app.borrow_mut().res.error = Some(crate::ui::guide::search_problem(
        crate::library::SearchError::Other,
    ));
    let mut h = album_harness(&app);
    h.run_steps(2);
    let text = "Couldn't search just now. Please try again in a moment.";
    assert_eq!(h.query_all_by_label(text).count(), 1);
    h.get_by_label("Try again");
}

#[test]
fn leaving_the_search_field_without_searching_records_nothing() {
    let d = tempfile::tempdir().unwrap();
    let app = searchable_app(d.path());
    let lib = Arc::clone(&app.borrow().lib);
    let mut h = album_harness(&app);
    h.get_by_label("Search your photos").click();
    h.run_steps(2);
    h.get_by_label("Search your photos").type_text("sunset");
    h.run_steps(2);
    // Before the pause in typing ends, Escape ends the typing.
    app.borrow_mut().query_changed_at = None;
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(lib.settings().recent_searches.is_empty());
    // So does clicking elsewhere.
    h.get_by_label("Search your photos").click();
    h.run_steps(2);
    app.borrow_mut().query_changed_at = None;
    h.get_by_label("Settings").click();
    h.run_steps(3);
    assert!(lib.settings().recent_searches.is_empty());
    // Enter does record.
    h.get_by_label("Close settings").click();
    h.run_steps(3);
    h.get_by_label("Search your photos").click();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(lib.settings().recent_searches, ["sunset"]);
}

#[test]
fn the_search_suggestions_work_with_the_keyboard() {
    let d = tempfile::tempdir().unwrap();
    let app = searchable_app(d.path());
    let mut h = album_harness(&app);
    let (first, second) = (
        crate::ui::guide::EXAMPLE_SEARCHES[0],
        crate::ui::guide::EXAMPLE_SEARCHES[1],
    );
    h.get_by_label("Search your photos").click();
    h.run_steps(3);
    h.get_by_label("Try");
    // Tab goes into the suggestions, which stay open.
    h.key_press(egui::Key::Tab);
    h.run_steps(4);
    h.get_by_label("Try");
    assert!(h.get_by_label(first).accesskit_node().is_focused());
    h.key_press(egui::Key::ArrowDown);
    h.run_steps(3);
    assert!(h.get_by_label(second).accesskit_node().is_focused());
    h.key_press(egui::Key::ArrowUp);
    h.run_steps(3);
    assert!(h.get_by_label(first).accesskit_node().is_focused());
    h.get_by_label("Try");
    // Enter runs the one that has focus.
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(app.borrow().query, first);
    // Escape closes them.
    app.borrow_mut().clear_search();
    h.run_steps(2);
    h.get_by_label("Search your photos").click();
    h.run_steps(3);
    h.key_press(egui::Key::Tab);
    h.run_steps(4);
    h.get_by_label("Try");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(h.query_by_label("Try searching for").is_none());
}

#[test]
fn asking_to_check_for_new_photos_while_busy_says_it_will_wait() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let lib = Arc::clone(&app.borrow().lib);
    let mut h = album_harness(&app);
    // A job held until `release` sends, or is dropped when the test fails (so it never hangs).
    let (release, hold) = std::sync::mpsc::channel::<()>();
    lib.spawn(move || {
        let _ = hold.recv();
    })
    .unwrap();
    app.borrow_mut().start_indexing();
    h.run_steps(3);
    h.get_by_label("Recollect will check for new photos when it finishes the current task.");
    release.send(()).unwrap();
    lib.wait();
    settle(&mut h, || lib.job_log.lock().contains(&"scan"));
    lib.wait();
}

#[test]
fn a_lost_background_answer_does_not_stall_the_snapshot_or_the_year_list() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let mut app = app.borrow_mut();
    // A snapshot thread that died without answering: the receiver is cleared, and a request
    // queued behind it goes out.
    let (tx, rx) = crossbeam_channel::bounded(1);
    app.snapshot_read.install(rx);
    app.snapshot_read.mark_again();
    drop(tx);
    app.poll_snapshot();
    assert!(
        !app.snapshot_read.again_queued(),
        "the queued refresh was started"
    );
    assert!(app.snapshot_read.is_pending(), "a fresh fetch is running");
    let fresh = app.snapshot_read.take_receiver().unwrap();
    drop(fresh);
    let (tx, rx) = crossbeam_channel::bounded(1);
    app.snapshot_read.install(rx);
    drop(tx);
    app.poll_snapshot();
    assert!(!app.snapshot_read.is_pending());
    // Same for the list of years.
    let (tx, rx) = crossbeam_channel::bounded::<Vec<i32>>(1);
    app.years_read.install(rx);
    app.years_seen = Some(7);
    drop(tx);
    app.poll_years(7);
    assert!(
        !app.years_read.is_pending(),
        "the failed lookup is forgotten"
    );
}

#[test]
fn the_timed_refresh_waits_for_the_answer_to_the_last_search() {
    let long_ago = Instant::now().checked_sub(Duration::from_secs(60)).unwrap();
    assert!(crate::ui::results_refresh_due(Some(long_ago), None));
    assert!(!crate::ui::results_refresh_due(Some(long_ago), Some(4)));
    assert!(!crate::ui::results_refresh_due(Some(Instant::now()), None));
    assert!(!crate::ui::results_refresh_due(None, None));
}

#[test]
fn a_slow_search_leaves_a_pause_after_its_answer_before_the_next_refresh() {
    // The search took longer than the refresh interval, but its answer arrived just now.
    let just_answered = Instant::now();
    assert!(!crate::ui::results_refresh_due(Some(just_answered), None));
}

#[test]
fn emptying_the_field_before_the_answer_arrives_leaves_no_results() {
    let d = tempfile::tempdir().unwrap();
    let app = searchable_app(d.path());
    let mut app = app.borrow_mut();
    let ctx = egui::Context::default();
    let paused = Instant::now().checked_sub(Duration::from_secs(5)).unwrap();
    app.query = "dog".into();
    app.query_changed_at = Some(paused);
    app.poll_debounce(&ctx);
    assert!(app.res.awaiting.is_some());
    app.query.clear();
    app.query_changed_at = Some(paused);
    app.poll_debounce(&ctx);
    // The answer to the old search arrives late and is ignored.
    for _ in 0..50 {
        app.poll_search();
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(app.res.awaiting.is_none());
    assert!(app.res.hits.is_empty());
    assert!(app.res.label.is_empty());
    assert!(app.res.error.is_none());
    assert!(!app.searching());
}

#[test]
fn clicking_a_tile_and_pressing_enter_opens_it() {
    let d = tempfile::tempdir().unwrap();
    let mut h = grid_harness(grid_app(d.path()));
    h.get_by_label("Result b.jpg").click();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    h.get_by_label("Back");
    h.get_by_label("b.jpg");
}

#[test]
fn enter_activates_a_focused_button_instead_of_opening_the_viewer() {
    let d = tempfile::tempdir().unwrap();
    let app = Rc::new(RefCell::new(grid_app(d.path())));
    let mut h = album_harness(&app);
    h.key_press(egui::Key::ArrowRight);
    h.run_steps(2);
    h.get_by_label("Clear search").focus();
    h.run_steps(2);
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert!(
        h.query_by_label("Back").is_none(),
        "the viewer stayed closed"
    );
    assert!(!app.borrow().searching(), "the search was cleared");
}
