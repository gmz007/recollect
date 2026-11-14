use super::entry::MomentChip;
use super::info::fmt_size;
use super::toolbar::position_slot;
use super::toolbar::toolbar_shows_text;
use super::*;
use crate::media::metadata::DateSource;
use crate::store::db::TimelineItem;

#[test]
fn sizes_read_naturally() {
    assert_eq!(fmt_size(0), "0 B");
    assert_eq!(fmt_size(1023), "1023 B");
    assert_eq!(fmt_size(2048), "2 KB");
    assert_eq!(fmt_size(3_355_443), "3.2 MB");
    assert_eq!(fmt_size(3 << 30), "3.0 GB");
}

fn entry(kind: MediaKind) -> ViewerEntry {
    ViewerEntry {
        file_id: 1,
        item_id: Some(10),
        path: "/x/clip.mp4".into(),
        kind,
        mtime: 5,
        taken_at: 0,
        date_source: None,
        duration: Some(100.0),
        width: Some(1920),
        height: Some(1080),
        favorite: false,
        camera: None,
        t: None,
        moments: Vec::new(),
    }
}

#[test]
fn video_frames_follow_the_moment_or_the_thumbnail_rule() {
    let mut v = entry(MediaKind::Video);
    assert_eq!(v.frame_time(), Some(5.0)); // 10 % of 100 s, capped at 5 s
    v.duration = Some(20.0);
    assert_eq!(v.frame_time(), Some(2.0));
    v.t = Some(42.5);
    assert_eq!(v.frame_time(), Some(42.5));
    assert_eq!(v.preview_key(1024, false).t_ms, Some(42_500));
    assert_eq!(v.thumb_key().t_ms, Some(42_500));
    assert_eq!(entry(MediaKind::Image).frame_time(), None);
}

#[test]
fn find_similar_starts_from_the_chosen_moment() {
    let mut v = entry(MediaKind::Video);
    v.moments = vec![
        MomentChip {
            item_id: 10,
            t_start: 4.0,
            t_end: 6.0,
            score: 0.3,
        },
        MomentChip {
            item_id: 11,
            t_start: 60.0,
            t_end: 62.0,
            score: 0.2,
        },
    ];
    v.t = Some(60.0);
    assert_eq!(v.similar_item(), Some(11));
    v.t = Some(4.0);
    assert_eq!(v.similar_item(), Some(10));
    v.t = None;
    assert_eq!(v.similar_item(), Some(10));
}

#[test]
fn navigation_stays_inside_the_list() {
    let mut v = Viewer::new(Source::Entries(vec![entry(MediaKind::Image); 3]), 1, true);
    v.zoom = true;
    v.go(2);
    assert_eq!(v.index, 2);
    assert!(!v.zoom, "moving on resets the zoom");
    v.go(99);
    assert_eq!(v.index, 2);
    v.go(0);
    assert_eq!(v.index, 0);
    let one = Source::Entries(vec![entry(MediaKind::Image)]);
    assert_eq!(Viewer::new(one, 7, true).index, 0);
}

fn timeline_item(i: i64) -> TimelineItem {
    TimelineItem {
        file_id: i,
        item_id: None,
        path: format!("/p/{i}.jpg").into(),
        kind: MediaKind::Image,
        mtime: 0,
        taken_at: i,
        date_source: DateSource::Exif,
        duration: None,
        width: None,
        height: None,
        favorite: false,
        camera: None,
    }
}

#[test]
fn a_huge_list_is_shared_not_copied() {
    let items: Arc<Vec<TimelineItem>> = Arc::new((0..200_000).map(timeline_item).collect());
    let mut v = Viewer::new(Source::Timeline(Arc::clone(&items)), 123_456, true);
    // The viewer holds the very same list; building the entry for the item on screen is
    // all it did per item.
    assert_eq!(Arc::strong_count(&items), 2);
    assert_eq!(v.current().file_id, 123_456);
    assert_eq!(v.len(), 200_000);
    v.go(7);
    assert_eq!(v.current().path, PathBuf::from("/p/7.jpg"));
    assert_eq!(Arc::strong_count(&items), 2);
    drop(v);
    assert_eq!(Arc::strong_count(&items), 1);
}

#[test]
fn favorite_marks_and_picked_moments_follow_the_viewer_across_entries() {
    let items: Arc<Vec<TimelineItem>> = Arc::new((0..3).map(timeline_item).collect());
    let mut v = Viewer::new(Source::Timeline(items), 0, true)
        .with_marks(HashMap::from([(2, true)]))
        .with_moment(1, 9.0);
    assert!(!v.current().favorite);
    v.set_favorite(0, true);
    assert!(v.current().favorite);
    v.go(1);
    assert_eq!(v.current().t, Some(9.0));
    v.go(2);
    assert!(v.current().favorite, "the mark given at the start applies");
    v.go(0);
    assert!(v.current().favorite, "and the one set while open");
}

#[test]
fn the_position_is_centred_beside_the_buttons_or_left_out() {
    // Plenty of room: in the middle.
    assert_eq!(position_slot(500.0, 100.0, 900.0, 60.0), Some(470.0));
    // The right-hand buttons reach past the middle: it moves next to them.
    assert_eq!(position_slot(500.0, 100.0, 520.0, 60.0), Some(448.0));
    // No gap wide enough: hidden rather than overlapping a button.
    assert_eq!(position_slot(500.0, 100.0, 150.0, 60.0), None);
    // Whatever the answer, it never touches either side.
    for right in (100..1000).step_by(7) {
        if let Some(x) = position_slot(500.0, 100.0, right as f32, 60.0) {
            assert!(x >= 112.0 && x + 60.0 <= right as f32 - 12.0);
        }
    }
}

#[test]
fn words_next_to_the_icons_need_a_wide_window() {
    assert!(!toolbar_shows_text(999.0));
    assert!(toolbar_shows_text(1000.0));
}
