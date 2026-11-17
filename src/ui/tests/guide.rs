//! Folders, activity, toasts, empty states, tips and wording.

use super::*;

#[test]
fn adding_a_folder_from_the_dialog_by_path() {
    let d = tempfile::tempdir().unwrap();
    let photos = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    fake_install(&lib);
    let mut h = harness(Arc::clone(&lib));
    h.get_all_by_label("Add a folder").next().unwrap().click();
    h.run_steps(3);
    h.get_by_label("Choose a folder…");
    h.get_by_label("or type a folder path");
    // A path that does not exist is refused with a plain message and the dialog stays.
    h.get_by_label("Folder path").click();
    h.run_steps(2);
    h.get_by_label("Folder path")
        .type_text("/definitely/not/here");
    h.run_steps(2);
    h.get_by_label("Add").click();
    h.run_steps(3);
    h.get_by_label_contains("couldn't be found");
    h.get_by_label("or type a folder path");
    // A real folder is added and the dialog closes.
    h.get_by_label("Folder path").click();
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by_label("Folder path")
        .type_text(photos.path().to_str().unwrap());
    h.run_steps(2);
    h.get_by_label("Add").click();
    settle(&mut h, || lib.folders().unwrap().len() == 1);
    h.run_steps(3);
    assert!(h.query_by_label("or type a folder path").is_none());
}

#[test]
fn stopping_a_folder_asks_first_and_keeps_the_files() {
    let d = tempfile::tempdir().unwrap();
    let (lib, app) = files_app(d.path(), 2);
    let mut h = album_harness(&app);
    settle(&mut h, || app.borrow().tl.items.len() == 2);
    h.get_by_label("Stop showing nowhere").click();
    h.run_steps(3);
    h.get_by_label("Stop showing “nowhere”?");
    h.get_by_label("Your files stay where they are.");
    assert_eq!(lib.folders().unwrap().len(), 1, "nothing happens yet");
    h.get_all_by_label("Cancel").last().unwrap().click();
    h.run_steps(3);
    assert!(
        h.query_by_label("Your files stay where they are.")
            .is_none()
    );
    assert_eq!(lib.folders().unwrap().len(), 1);
    h.get_by_label("Stop showing nowhere").click();
    h.run_steps(3);
    h.get_by_label("Stop showing").click();
    settle(&mut h, || lib.folders().unwrap().is_empty());
}

#[test]
fn the_activity_card_describes_what_is_happening() {
    use crate::library::{Activity, IndexProgress};
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    let lib = Arc::clone(&app.borrow().lib);
    assert!(h.query_by_label_contains("Looking for").is_none());
    let show = |h: &mut Harness<'_>, a: Activity| {
        lib.update_status(|s| s.activity = a);
        h.run_steps(2);
    };
    show(
        &mut h,
        Activity::Scanning {
            folder: "/x".into(),
        },
    );
    // The foot of the sidebar shows a ring; its details open on a click.
    assert!(h.query_by_label("Pause").is_none());
    h.get_by_label("Progress: Looking for new photos…").click();
    h.run_steps(2);
    h.get_by_label("Looking for new photos…");
    h.get_by_label("Pause");
    h.get_by_label("Stop");
    // Paused work says so in the foot, not only in the details.
    lib.update_status(|s| s.paused = true);
    h.run_steps(2);
    h.get_by_label("Progress: Paused — Looking for new photos…");
    lib.update_status(|s| s.paused = false);
    h.run_steps(2);
    show(
        &mut h,
        Activity::ReadingDates {
            done: 1234,
            total: 5000,
        },
    );
    h.get_by_label("Reading photo dates… 1,234 of 5,000");
    show(
        &mut h,
        Activity::Indexing(IndexProgress {
            done: 45,
            total: 100,
            ..Default::default()
        }),
    );
    h.get_by_label("Getting your photos ready for search… 45 %");
    show(
        &mut h,
        Activity::Downloading {
            model: "m".into(),
            done: 120_000_000,
            total: 485_000_000,
        },
    );
    h.get_by_label("Downloading smart search… 120 of 485 MB");
    assert!(
        h.query_by_label("Pause").is_none(),
        "a download is only cancelled"
    );
    show(&mut h, Activity::Idle);
    assert!(h.query_by_label_contains("Downloading").is_none());
}

#[test]
fn notices_appear_as_toasts_and_errors_can_be_closed() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    app.borrow_mut().notify("Added 2 items to “Trip”");
    h.run_steps(2);
    h.get_by_label("Added 2 items to “Trip”");
    // Information fades by itself, and can also be closed.
    h.get_by_label("Dismiss").click();
    h.run_steps(2);
    assert!(h.query_by_label("Added 2 items to “Trip”").is_none());
    app.borrow_mut().notify("Added 3 items to “Trip”");
    h.run_steps(2);
    h.get_by_label("Added 3 items to “Trip”");
    app.borrow_mut()
        .notify_error("That didn't work. Please try again.");
    h.run_steps(2);
    h.get_by_label("That didn't work. Please try again.");
    h.get_by_label("Dismiss").click();
    h.run_steps(2);
    assert!(
        h.query_by_label("That didn't work. Please try again.")
            .is_none()
    );
}

#[test]
fn help_has_a_section_for_each_question() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    h.get_by_label("Help").click();
    h.run_steps(3);
    // Getting started comes first.
    h.get_by_label("Add your photo folders");
    h.get_by_label("Search or browse");
    for (topic, heading, text) in [
        (
            "Searching",
            "Describe what you remember",
            "kids on the beach",
        ),
        ("Searching", "Narrow the results", "When"),
        ("Searching", "Moments in videos", "at 1:23"),
        ("Organizing", "Favorites", "heart"),
        ("Organizing", "Albums", "Add to album"),
        ("Organizing", "Selecting several photos", "circle"),
        ("Keyboard shortcuts", "Arrow keys", "Move between photos"),
        (
            "Privacy",
            "Everything stays on this computer",
            "never sent anywhere",
        ),
    ] {
        h.get_by_label(&format!("{topic} help")).click();
        h.run_steps(3);
        // (Favorites and Albums are also names in the sidebar behind the sheet.)
        assert!(h.query_all_by_label(heading).next().is_some(), "{heading}");
        assert!(
            h.query_all_by_label_contains(text).next().is_some(),
            "{text}"
        );
    }
    h.get_by_label("Close help").click();
    h.run_steps(3);
    assert!(!app.borrow().dialogs.help_open);
    // Esc closes it too.
    h.get_by_label("Help").click();
    h.run_steps(3);
    h.key_press(egui::Key::Escape);
    h.run_steps(3);
    assert!(!app.borrow().dialogs.help_open);
}

#[test]
fn the_banned_word_check_works() {
    let t = vec!["Download the Model".to_string(), "500 ms".to_string()];
    assert_eq!(banned_words_in(&t).len(), 2);
    let ok = vec!["Months".to_string(), "Smart search is off".to_string()];
    assert!(banned_words_in(&ok).is_empty());
}

/// The main window with items: every state of the activity card, selection mode, toasts,
/// Settings, Help, the search help and the tips.
fn check_main_window(check: &mut dyn FnMut(&str, &Harness<'_>), dir: &std::path::Path) {
    use crate::library::Activity;
    let app = album_app(dir, true);
    let mut h = album_harness(&app);
    check("timeline", &h);
    // Activity in each state.
    let mut details_open = false;
    let lib = Arc::clone(&app.borrow().lib);
    for a in [
        Activity::Scanning {
            folder: "/x".into(),
        },
        Activity::ReadingDates { done: 1, total: 2 },
        Activity::LoadingModel,
        Activity::Indexing(crate::library::IndexProgress {
            done: 1,
            total: 2,
            ..Default::default()
        }),
        Activity::Downloading {
            model: "m".into(),
            done: 1,
            total: 2,
        },
    ] {
        lib.update_status(|s| s.activity = a);
        h.run_steps(2);
        check("activity", &h);
        if !details_open {
            // The details (with Pause and Stop) open from the ring at the foot.
            h.get_by_label_contains("Progress:").click();
            h.run_steps(2);
            check("activity details", &h);
            details_open = true;
        }
    }
    lib.update_status(|s| s.activity = Activity::Idle);
    h.run_steps(2);
    // Selection mode.
    h.get_by_label("Select img2.jpg").click();
    h.run_steps(3);
    h.get_by_label("1 selected");
    check("selection mode", &h);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    // Notices.
    app.borrow_mut()
        .notify_error("Couldn't save your favorite. Try again in a moment.");
    h.run_steps(2);
    check("error toast", &h);
    // Settings, every section. The Advanced and Details parts stay folded away.
    {
        let mut a = app.borrow_mut();
        a.snapshot.stats.images = 240;
        a.snapshot.stats.indexed = 238;
        a.snapshot.stats.errors = 2;
        a.snapshot.errors = vec![("/nowhere/x.jpg".into(), "unreadable".into())];
        a.failed_open = true;
    }
    for section in ["Appearance", "Smart search", "Library", "Storage", "About"] {
        open_section(&mut h, section);
        check(&format!("settings: {section}"), &h);
    }
    open_section(&mut h, "Smart search");
    h.get_by_label("Best quality").click();
    h.run_steps(3);
    h.get_by_label("Switch to Best quality?");
    check("settings: switching", &h);
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    h.get_by_label("Close settings").click();
    h.run_steps(3);
    // Help.
    h.get_by_label("Help").click();
    h.run_steps(3);
    for topic in [
        "Getting started",
        "Searching",
        "Organizing",
        "Keyboard shortcuts",
        "Privacy",
    ] {
        h.get_by_label(&format!("{topic} help")).click();
        h.run_steps(3);
        check(&format!("help: {topic}"), &h);
    }
    h.get_by_label("Close help").click();
    h.run_steps(3);
    // The search help, with a recent search in it, and the tips.
    app.borrow_mut().remember_search("snow");
    app.borrow_mut().snapshot.model_installed = true;
    h.get_by_label("Search your photos").click();
    h.run_steps(3);
    h.get_by_label("kids on the beach");
    check("search help", &h);
    app.borrow_mut().search_help.focused = false;
    app.borrow_mut().search_help.field = None;
    for tip in [crate::ui::guide::Tip::Search, crate::ui::guide::Tip::Select] {
        h.run_steps(3);
        h.get_by_label(tip.text());
        check("tip", &h);
        h.get_by_label("Got it").click();
        h.run_steps(3);
    }
}

#[test]
fn the_main_screens_use_no_technical_words() {
    let d = tempfile::tempdir().unwrap();
    let mut seen: Vec<String> = Vec::new();
    let mut check = |name: &str, h: &Harness<'_>| {
        let texts = visible_text(h);
        assert!(!texts.is_empty(), "{name}: nothing to read");
        let bad = banned_words_in(&texts);
        assert!(bad.is_empty(), "{name}: {bad:?}");
        seen.extend(texts);
    };

    // Empty library.
    {
        let d = tempfile::tempdir().unwrap();
        let lib = library(d.path());
        fake_install(&lib);
        let h = harness(lib);
        check("empty library", &h);
    }
    // The welcome, every screen.
    {
        let d = tempfile::tempdir().unwrap();
        let app = fresh_app(d.path());
        let mut h = album_harness(&app);
        check("welcome: first screen", &h);
        h.get_by_label("Get started").click();
        h.run_steps(3);
        check("welcome: folders", &h);
        h.get_by_label("Continue without folders").click();
        h.run_steps(3);
        check("welcome: smart search", &h);
        h.get_by_label("Not now").click();
        h.run_steps(3);
        check("welcome: all set", &h);
    }
    // Every empty state.
    for (name, which) in EMPTY_VIEWS {
        let d = tempfile::tempdir().unwrap();
        let (_app, mut h) = empty_view(d.path(), which);
        h.run_steps(2);
        check(name, &h);
    }
    // Timeline with items, smart search off (banner, invitation), then on.
    {
        let app = album_app(d.path(), false);
        let h = album_harness(&app);
        check("timeline, smart search off", &h);
    }
    {
        let d1 = tempfile::tempdir().unwrap();
        let app = album_app(d1.path(), false);
        let mut h = album_harness(&app);
        open_section(&mut h, "Smart search");
        h.get_by_label("Download and turn on (608 MB)");
        check("settings: smart search off", &h);
    }
    {
        let d2 = tempfile::tempdir().unwrap();
        check_main_window(&mut check, d2.path());
    }
    // Search results and an empty result.
    {
        let d3 = tempfile::tempdir().unwrap();
        let mut app = grid_app(d3.path());
        let results = Arc::make_mut(&mut app.res.hits);
        results[0] = hit("/nowhere/movie.mp4", MediaKind::Video, Some(42.0));
        let app = Rc::new(RefCell::new(app));
        let mut h = album_harness(&app);
        check("search results", &h);
        let mut a = app.borrow_mut();
        a.res.hits = Arc::default();
        drop(a);
        h.run_steps(2);
        check("no results", &h);
    }
    // The viewer, with its labelled toolbar and the details panel.
    {
        let d5 = tempfile::tempdir().unwrap();
        let app = album_app(d5.path(), true);
        let mut h = quick_harness(&app);
        open_tile(&mut h, "Item img1.jpg");
        h.run_steps(3);
        check("viewer", &h);
    }
    // The "On this day" banner, the size chip and Settings > Appearance with the photo size.
    {
        let d6 = tempfile::tempdir().unwrap();
        let app = album_app(d6.path(), true);
        app.borrow_mut().today_override = chrono::NaiveDate::from_ymd_opt(2026, 3, 15);
        let mut h = album_harness(&app);
        h.get_by_label("On this day");
        check("on this day banner", &h);
        h.key_combination_modifiers(egui::Modifiers::COMMAND, &[egui::Key::Equals]);
        h.run_steps(3);
        h.get_by_label("Photo size");
        check("photo size chip", &h);
        open_section(&mut h, "Appearance");
        h.get_all_by_label("Photo size").next().unwrap();
        check("settings: appearance", &h);
    }
    // An album view.
    {
        let d4 = tempfile::tempdir().unwrap();
        let (_lib, _app, h) = album_view(d4.path(), 3, album_harness);
        check("album", &h);
    }
    // Something was actually read.
    assert!(seen.iter().any(|t| t == "Recently added"));
    assert!(
        seen.iter()
            .any(|t| t.contains("Getting your photos ready for search"))
    );
}

#[test]
fn the_skipped_folders_notice_shows_when_a_run_ends() {
    use crate::library::RunSummary;
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    let lib = Arc::clone(&app.borrow().lib);
    lib.update_status(|s| {
        s.last_run = Some(RunSummary {
            skipped_folders: vec!["/gone".into()],
            ..Default::default()
        });
        // A finished run is counted, whether or not the app ever saw it running.
        s.run_seq += 1;
    });
    h.run_steps(2);
    h.get_by_label("Some folders couldn't be reached, so they were left as they were.");
    // Without a run that just ended it stays quiet.
    app.borrow_mut().notices.dismiss_current();
    h.run_steps(3);
    assert!(app.borrow().notices.current().is_none());
}

const SOMETHING_WRONG: &str =
    "Something went wrong. Please try again, and restart Recollect if it keeps happening.";

#[test]
fn an_identical_error_toasts_again() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    let lib = Arc::clone(&app.borrow().lib);
    lib.update_status(|s| s.set_error("disk full"));
    h.run_steps(2);
    h.get_by_label(SOMETHING_WRONG);
    h.get_by_label("Dismiss").click();
    h.run_steps(2);
    assert!(h.query_by_label_contains("Something went wrong").is_none());
    // Nothing new: no new toast.
    h.run_steps(3);
    assert!(h.query_by_label_contains("Something went wrong").is_none());
    // The same text again is a new failure.
    lib.update_status(|s| s.set_error("disk full"));
    h.run_steps(2);
    h.get_by_label(SOMETHING_WRONG);
}

#[test]
fn folder_suggestions_are_worked_out_once_when_the_dialog_opens() {
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let mut h = album_harness(&app);
    let candidates = vec![
        (
            "Pictures".to_string(),
            std::path::PathBuf::from("/home/a/Pictures"),
        ),
        (
            "Videos".to_string(),
            std::path::PathBuf::from("/home/a/Videos"),
        ),
    ];
    app.borrow_mut().open_add_folder_with(candidates);
    h.run_steps(3);
    h.get_by_label("Add Pictures");
    h.get_by_label("Add Videos");
    // The library changes while the dialog is open: the list is not recomputed.
    app.borrow_mut()
        .snapshot
        .folders
        .push(crate::store::db::Folder {
            id: 2,
            path: "/home/a/Pictures".into(),
        });
    h.run_steps(3);
    h.get_by_label("Add Pictures");
    assert_eq!(
        app.borrow()
            .dialogs
            .add_folder_dialog
            .as_ref()
            .unwrap()
            .suggestion_count(),
        2
    );
}

#[test]
fn every_view_has_an_empty_state_with_a_sentence_and_a_button() {
    let cases = [
        (
            EmptyView::NoFolders,
            "Add a folder with your photos and videos to get started.",
            "Add a folder",
        ),
        (
            EmptyView::Photos,
            "No photos or videos yet. Add a folder, or give Recollect a moment to look through \
             the ones you have.",
            "Add a folder",
        ),
        (
            EmptyView::Recent,
            "Photos and videos you add to your folders show up here.",
            "Check for new photos",
        ),
        (
            EmptyView::Favorites,
            "Click the heart on any photo to add it here.",
            "Go to Photos",
        ),
        (
            EmptyView::Folder,
            "There are no photos or videos in this folder.",
            "Check for new photos",
        ),
        (
            EmptyView::Album,
            "This album is empty. Select photos and choose Add to album.",
            "Go to Photos",
        ),
        (
            EmptyView::NoResults,
            "No photos match “snow”. Try different words.",
            "Start over",
        ),
        (
            EmptyView::NoResultsDated,
            "No photos match “snow”. Try different words, or check the When filter.",
            "Clear the date filter",
        ),
        (
            EmptyView::SearchOff,
            "Smart search is off. Turn it on to find photos by describing them.",
            "Turn on smart search",
        ),
    ];
    for (which, sentence, button) in cases {
        let d = tempfile::tempdir().unwrap();
        let (_app, h) = empty_view(d.path(), which);
        h.get_by_label(sentence);
        h.get_all_by_label(button).last().unwrap();
    }
}

#[test]
fn the_buttons_of_the_empty_states_work() {
    // Favorites and album: "Go to Photos".
    let d = tempfile::tempdir().unwrap();
    let (app, mut h) = empty_view(d.path(), EmptyView::Favorites);
    h.get_by_label("Go to Photos").click();
    h.run_steps(3);
    assert_eq!(app.borrow().tl.view, View::Timeline);
    // No results: clearing the date filter keeps the words; clearing the search drops them.
    let d = tempfile::tempdir().unwrap();
    let (app, mut h) = empty_view(d.path(), EmptyView::NoResultsDated);
    h.get_by_label("Clear the date filter").click();
    h.run_steps(3);
    assert_eq!(app.borrow().date, crate::ui::when::DateFilter::default());
    assert!(app.borrow().res.current.is_some());
    let d = tempfile::tempdir().unwrap();
    let (app, mut h) = empty_view(d.path(), EmptyView::NoResults);
    h.get_by_label("Start over").click();
    h.run_steps(3);
    assert!(app.borrow().res.current.is_none() && app.borrow().query.is_empty());
    // Search off: opens Settings on smart search.
    let d = tempfile::tempdir().unwrap();
    let (app, mut h) = empty_view(d.path(), EmptyView::SearchOff);
    h.get_by_label("Turn on smart search").click();
    h.run_steps(3);
    assert!(app.borrow().dialogs.settings_open);
    assert_eq!(
        app.borrow().dialogs.settings_section,
        crate::ui::settings::Section::SmartSearch
    );
}

#[test]
fn tips_show_one_at_a_time_once_each() {
    use crate::ui::guide::Tip;
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), true);
    let lib = Arc::clone(&app.borrow().lib);
    let mut h = album_harness(&app);
    // Photos and smart search are there: the search tip comes first, alone.
    h.get_by_label(Tip::Search.text());
    assert!(h.query_by_label(Tip::Select.text()).is_none());
    assert_eq!(h.query_all_by_label("Got it").count(), 1);
    h.get_by_label("Got it").click();
    h.run_steps(3);
    h.get_by_label(Tip::Select.text());
    assert!(h.query_by_label(Tip::Search.text()).is_none());
    h.get_by_label("Got it").click();
    h.run_steps(3);
    assert!(h.query_by_label("Got it").is_none());
    assert_eq!(lib.settings().tips_dismissed, ["search", "select"]);
    // After a restart they stay dismissed.
    drop(h);
    let mut restarted = App::new(&egui::Context::default(), Arc::clone(&lib));
    restarted.frame_stats = None;
    restarted.snapshot.folders = vec![crate::store::db::Folder {
        id: 1,
        path: "/nowhere".into(),
    }];
    restarted.snapshot.model_installed = true;
    restarted.inject_timeline(spread_items());
    let restarted = Rc::new(RefCell::new(restarted));
    let mut h = album_harness(&restarted);
    assert!(h.query_by_label("Got it").is_none());
    // Settings can bring them back.
    open_section(&mut h, "Appearance");
    h.get_by_label("Show tips again").click();
    h.run_steps(3);
    assert!(lib.settings().tips_dismissed.is_empty());
    h.get_by_label("Close settings").click();
    h.run_steps(3);
    h.get_by_label(Tip::Search.text());
}

#[test]
fn without_smart_search_the_first_tip_is_about_selecting() {
    use crate::ui::guide::Tip;
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let mut h = album_harness(&app);
    // The offer to turn on smart search comes first; no tip stacks on it.
    h.get_by_label("Not now").click();
    h.run_steps(3);
    h.get_by_label(Tip::Select.text());
    assert!(h.query_by_label(Tip::Search.text()).is_none());
}

#[test]
fn only_one_banner_shows_above_the_grid_at_a_time() {
    use crate::ui::guide::Tip;
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let mut h = album_harness(&app);
    // The smart search offer is showing: no tip next to it.
    h.get_by_label("Turn on smart search to find photos by describing them");
    for tip in [Tip::Select, Tip::Search, Tip::Album] {
        assert!(h.query_by_label(tip.text()).is_none(), "{tip:?}");
    }
    // Once it is dealt with, the tip takes its place.
    h.get_by_label("Not now").click();
    h.run_steps(3);
    assert!(
        h.query_by_label("Turn on smart search to find photos by describing them")
            .is_none()
    );
    h.get_by_label(Tip::Select.text());
}

#[test]
fn the_first_album_view_explains_how_to_fill_it() {
    use crate::ui::guide::Tip;
    let d = tempfile::tempdir().unwrap();
    let (lib, _app, mut h) = album_view(d.path(), 3, album_harness);
    // The smart search offer has been dealt with, so it does not hold the tip back.
    lib.update_settings(|s| s.search_prompt_dismissed = true)
        .unwrap();
    h.run_steps(3);
    h.get_by_label(Tip::Album.text());
    assert!(
        h.query_by_label(Tip::Select.text()).is_none(),
        "one at a time"
    );
    h.get_by_label("Got it").click();
    h.run_steps(3);
    assert!(lib.settings().tips_dismissed.contains(&"album".to_string()));
    assert!(h.query_by_label(Tip::Album.text()).is_none());
}

#[test]
fn a_failed_download_says_what_to_do_and_keeps_the_details_folded() {
    use crate::library::ErrorKind;
    let d = tempfile::tempdir().unwrap();
    let app = album_app(d.path(), false);
    let mut h = album_harness(&app);
    let lib = Arc::clone(&app.borrow().lib);
    lib.update_status(|s| {
        s.set_error_of(
            ErrorKind::Download,
            "Model download failed: dns error: failed to lookup address",
        );
    });
    h.run_steps(3);
    h.get_by_label("Couldn't download smart search. Check your internet connection and try again.");
    h.get_all_by_label("Try again").next().unwrap();
    assert!(h.query_by_label_contains("dns error").is_none(), "folded");
    h.get_by_label("Details").click();
    h.run_steps(3);
    h.get_by_label_contains("dns error");
    // Other failures are explained in the same way.
    for (kind, text) in [
        (ErrorKind::AddFolder, "Couldn't add that folder."),
        (
            ErrorKind::Scan,
            "Couldn't finish looking through your photos.",
        ),
        (ErrorKind::SwitchModel, "Couldn't change smart search."),
    ] {
        let (msg, _) = crate::ui::guide::describe_failure(kind);
        assert!(msg.starts_with(text), "{msg}");
        assert!(msg.ends_with('.') && msg.contains("try") || msg.contains("Try"));
    }
}

#[test]
fn other_messages_say_what_happened_and_what_to_do() {
    use crate::library::SearchError;
    use crate::store::db::AlbumError;
    let (text, details) = crate::ui::guide::album_problem(
        Some(&AlbumError::NameTaken("Trip".into())),
        "An album named “Trip” already exists",
    );
    assert_eq!(
        text,
        "An album named “Trip” already exists. Choose a different name."
    );
    assert_eq!(details, None);
    let (text, details) = crate::ui::guide::album_problem(Some(&AlbumError::NameEmpty), "");
    assert_eq!(text, "An album needs a name. Type a name and try again.");
    assert_eq!(details, None);
    let (text, details) = crate::ui::guide::album_problem(None, "database is locked");
    assert_eq!(
        text,
        "Couldn't save that album change. Nothing was changed. Please try again."
    );
    assert_eq!(details.as_deref(), Some("database is locked"));
    let p = crate::ui::guide::search_problem(SearchError::ModelMissing);
    assert!(p.text.starts_with("Search isn't ready yet.") && p.can_retry);
    let p = crate::ui::guide::search_problem(SearchError::Other);
    assert!(p.text.starts_with("Couldn't search just now.") && p.can_retry);
    let p = crate::ui::guide::search_problem(SearchError::ItemGone);
    assert_eq!(p.text, "That photo is no longer in your library.");
    assert!(!p.can_retry);
}

#[test]
fn errors_are_told_apart_by_kind_not_by_their_text() {
    use crate::library::SearchError;
    use crate::store::db::{AlbumError, check_album_name};
    let d = tempfile::tempdir().unwrap();
    let lib = library(d.path());
    // No model installed.
    let err = lib
        .search("dog", &crate::search::SearchOptions::default())
        .unwrap_err();
    assert_eq!(SearchError::of(&err), SearchError::ModelMissing);
    // An item that is not there.
    let err = lib
        .similar(12345, &crate::search::SearchOptions::default())
        .unwrap_err();
    assert_eq!(SearchError::of(&err), SearchError::ItemGone);
    assert_eq!(check_album_name("", [], None), Err(AlbumError::NameEmpty));
    assert_eq!(
        check_album_name("Trip", [(1, "trip")], None),
        Err(AlbumError::NameTaken("trip".into()))
    );
    assert_eq!(
        check_album_name(&"x".repeat(500), [], None),
        Err(AlbumError::TooLong)
    );
    // Through the writer thread, the kind arrives with the failure.
    lib.create_album("Trip", &[]).unwrap();
    let t = lib.queue_album(crate::library::AlbumOp::Create {
        name: "trip".into(),
        add: vec![],
    });
    lib.wait_writes();
    let w = lib
        .take_album_results()
        .into_iter()
        .find(|w| w.ticket == t)
        .unwrap();
    assert!(matches!(w.error_kind, Some(AlbumError::NameTaken(_))));
}

#[test]
fn a_removed_photo_in_find_similar_has_no_try_again() {
    let d = tempfile::tempdir().unwrap();
    let app = searchable_app(d.path());
    app.borrow_mut().res.current = Some(crate::ui::Query::Similar {
        item_id: 999,
        label: "a.jpg".into(),
    });
    app.borrow_mut().res.label = "similar to a.jpg".into();
    app.borrow_mut().res.error = Some(crate::ui::guide::search_problem(
        crate::library::SearchError::ItemGone,
    ));
    let mut h = album_harness(&app);
    h.run_steps(2);
    // Said once: in the empty state, not also in the header.
    assert_eq!(
        h.query_all_by_label("That photo is no longer in your library.")
            .count(),
        1
    );
    assert!(h.query_by_label("Try again").is_none());
    h.get_by_label("Start over");
}
