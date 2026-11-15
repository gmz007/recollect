//! The smart-search choice, settings and the status.

use super::*;

#[test]
fn model_switch_rebuilds_index_and_old_vectors_are_rejected() {
    let d = tempfile::tempdir().unwrap();
    let item = seed(&open(d.path()));
    let lib = open(d.path()); // reload: the index now holds the seeded vector
    assert_eq!(lib.indexed_items(), 1);
    assert!(
        lib.similar(item, &SearchOptions::default())
            .unwrap()
            .is_empty()
    );

    lib.set_model(registry::SIGLIP_B16.id).unwrap();
    lib.wait();
    assert_eq!(lib.status().last_error, None);
    assert_eq!(lib.model().id, registry::SIGLIP_B16.id);
    assert_eq!(lib.indexed_items(), 0);
    let saved = Settings::load(&lib.paths().settings_path()).unwrap();
    assert_eq!(saved.model.as_deref(), Some(registry::SIGLIP_B16.id));
    // The old 512-d vector must produce an error, not a panic.
    let err = lib.similar(item, &SearchOptions::default()).unwrap_err();
    assert_eq!(SearchError::of(&err), SearchError::NotReady, "{err}");
    assert!(search::by_vector(&lib, &[0.0; 512], &SearchOptions::default(), None).is_err());
}

#[test]
fn a_settings_change_during_a_model_switch_is_kept() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    {
        // The switch job waits for the database while the user changes the theme.
        let _db = lib.db.lock();
        lib.set_model(registry::SIGLIP_B16.id).unwrap();
        lib.update_settings(|s| {
            s.theme = crate::config::Theme::Dark;
            s.use_gpu = false;
        })
        .unwrap();
    }
    lib.wait();
    assert_eq!(lib.status().last_error, None);
    assert_eq!(lib.model().id, registry::SIGLIP_B16.id);
    let saved = Settings::load(&lib.paths().settings_path()).unwrap();
    assert_eq!(saved.model.as_deref(), Some(registry::SIGLIP_B16.id));
    assert_eq!(saved.theme, crate::config::Theme::Dark);
    assert!(!saved.use_gpu);
    assert_eq!(lib.settings(), saved);
}

#[test]
fn update_settings_leaves_the_model_choice_alone() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    let before = lib.settings().model;
    lib.update_settings(|s| {
        s.model = Some("something-else".into());
        s.search_prompt_dismissed = true;
    })
    .unwrap();
    assert_eq!(lib.settings().model, before);
    assert!(lib.settings().search_prompt_dismissed);
    let saved = Settings::load(&lib.paths().settings_path()).unwrap();
    assert!(saved.search_prompt_dismissed);
}

#[test]
fn every_recorded_error_is_counted_even_when_the_text_repeats() {
    let d = tempfile::tempdir().unwrap();
    let lib = open(d.path());
    assert_eq!(lib.status().error_seq, 0);
    lib.update_status(|s| s.set_error("disk full"));
    let first = lib.status().error_seq;
    lib.update_status(|s| s.set_error("disk full"));
    let st = lib.status();
    assert_eq!(st.last_error.as_deref(), Some("disk full"));
    assert_eq!(st.error_seq, first + 1);
}
