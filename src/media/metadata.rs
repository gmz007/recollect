//! Capture dates and camera names, for ordering photos on a timeline.
//!
//! A capture time is stored as the *local wall-clock time* at the place of capture, encoded as
//! if it were UTC. Cameras write EXIF times without a time zone, and phones write video times
//! with one; what people remember ("we were there at 3 pm") is the wall-clock reading, so that
//! is what we keep. Sorting and grouping by day then never depends on the viewer's time zone.

use super::MediaKind;
use super::video;
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// Where a capture time came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DateSource {
    /// The file's modification time (nothing better was found).
    File,
    Exif,
    Video,
}

impl DateSource {
    pub fn as_i64(self) -> i64 {
        match self {
            DateSource::File => 0,
            DateSource::Exif => 1,
            DateSource::Video => 2,
        }
    }

    pub fn from_i64(v: i64) -> Self {
        match v {
            1 => DateSource::Exif,
            2 => DateSource::Video,
            _ => DateSource::File,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Capture {
    /// Local wall-clock capture time, encoded as seconds since the epoch as if it were UTC.
    pub taken_at: i64,
    pub source: DateSource,
    pub camera: Option<String>,
    /// Length in seconds (videos only), read from the container header.
    pub duration: Option<f64>,
}

/// 1971-01-01: earlier values are zeroed or reset clocks, not real capture times.
const MIN_PLAUSIBLE: i64 = 365 * 86_400;

/// Converts a real instant (UTC seconds) to the wall-clock encoding in the machine's time zone.
pub fn local_wall_clock(utc_secs: i64) -> i64 {
    match Local.timestamp_opt(utc_secs, 0).single() {
        Some(t) => t.naive_local().and_utc().timestamp(),
        None => utc_secs,
    }
}

/// The capture time every list, filter and sort uses for a file: the stored date, or, while
/// none is known, the modification time `mtime` (UTC seconds) in local wall-clock encoding.
/// All readers of the `files` table go through this one rule so a file has the same date
/// everywhere.
pub fn effective_taken(taken_at: Option<i64>, mtime: i64) -> i64 {
    taken_at.unwrap_or_else(|| local_wall_clock(mtime))
}

fn plausible(wall: i64) -> bool {
    wall >= MIN_PLAUSIBLE && wall <= local_wall_clock(Utc::now().timestamp()) + 86_400
}

/// TIFF-based files (TIFF, DNG, many raw formats) larger than this are not parsed for EXIF:
/// the EXIF reader loads a whole TIFF into memory (it only reads other containers' metadata
/// segment), which would cost hundreds of MB per decode thread for a date.
const MAX_TIFF_EXIF_BYTES: u64 = 64 << 20;

/// Reads the best available capture time of a file. When the file opens but has no usable
/// metadata, the modification time `mtime` (UTC seconds) is used. Returns `None` when the file
/// cannot be opened or read right now (missing, unmounted drive, I/O error): the caller should
/// try again later instead of settling for a guess.
pub fn read_capture(path: &Path, kind: MediaKind, mtime: i64) -> Option<Capture> {
    let fallback = |duration| Capture {
        taken_at: effective_taken(None, mtime),
        source: DateSource::File,
        camera: None,
        duration,
    };
    match kind {
        MediaKind::Image => match read_exif(path) {
            Ok(found) => Some(found.unwrap_or_else(|| fallback(None))),
            Err(e) => {
                tracing::warn!("{}: {e}; reading its date again next time", path.display());
                None
            }
        },
        MediaKind::Video => {
            // Check readability ourselves so a vanished file is not mistaken for "no tags".
            if let Err(e) = File::open(path) {
                tracing::warn!("{}: {e}; reading its date again next time", path.display());
                return None;
            }
            let (found, duration) = read_video(path);
            Some(found.unwrap_or_else(|| fallback(duration)))
        }
    }
}

fn ascii(field: &exif::Field) -> Option<String> {
    match &field.value {
        exif::Value::Ascii(v) => {
            let s = String::from_utf8_lossy(v.first()?);
            let s = s.trim_matches(|c: char| c == '\0' || c.is_whitespace());
            (!s.is_empty()).then(|| s.to_string())
        }
        _ => None,
    }
}

/// `Err` only for I/O trouble; a file without usable EXIF is `Ok(None)`.
fn read_exif(path: &Path) -> std::io::Result<Option<Capture>> {
    use std::io::{Read, Seek, SeekFrom};
    let file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut reader = BufReader::new(file);
    let mut magic = [0u8; 4];
    match reader.read_exact(&mut magic) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    if matches!(&magic, b"II*\0" | b"MM\0*") && len > MAX_TIFF_EXIF_BYTES {
        return Ok(None);
    }
    reader.seek(SeekFrom::Start(0))?;
    let ex = match exif::Reader::new().read_from_container(&mut reader) {
        Ok(ex) => ex,
        Err(exif::Error::Io(e)) if e.kind() != std::io::ErrorKind::UnexpectedEof => return Err(e),
        Err(_) => return Ok(None),
    };
    let text = |tag| ex.get_field(tag, exif::In::PRIMARY).and_then(ascii);
    let make = text(exif::Tag::Make);
    let model = text(exif::Tag::Model);
    let camera = camera_name(make.as_deref(), model.as_deref());
    // The first plausible one wins; "0000:00:00 00:00:00" and friends are skipped.
    let taken_at = [
        exif::Tag::DateTimeOriginal,
        exif::Tag::DateTimeDigitized,
        exif::Tag::DateTime,
    ]
    .into_iter()
    .filter_map(text)
    .filter_map(|s| NaiveDateTime::parse_from_str(&s, "%Y:%m:%d %H:%M:%S").ok())
    .map(|n| n.and_utc().timestamp())
    .find(|&t| plausible(t));
    Ok(taken_at.map(|taken_at| Capture {
        taken_at,
        source: DateSource::Exif,
        camera,
        duration: None,
    }))
}

/// Longest make or model kept, in characters: camera strings come from untrusted tags and end
/// up in the database and the UI.
const MAX_CAMERA_CHARS: usize = 128;

/// "Make Model", without repeating the make when the model already starts with it
/// ("Canon" + "Canon EOS 5D" is just "Canon EOS 5D"). Each part is cut to
/// [`MAX_CAMERA_CHARS`] characters.
fn camera_name(make: Option<&str>, model: Option<&str>) -> Option<String> {
    let bound = |s: &str| s.chars().take(MAX_CAMERA_CHARS).collect::<String>();
    match (make.map(bound), model.map(bound)) {
        (Some(mk), Some(md)) if md.to_lowercase().starts_with(&mk.to_lowercase()) => Some(md),
        (Some(mk), Some(md)) => Some(format!("{mk} {md}")),
        (Some(s), None) | (None, Some(s)) => Some(s),
        (None, None) => None,
    }
}

/// Parses a container timestamp. Values with an offset keep their own wall-clock part (the
/// time on the clock where it was filmed); values without one are UTC and are converted to
/// this machine's local time.
fn parse_video_time(s: &str) -> Option<i64> {
    let s = s.trim();
    let wall = if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        if s.ends_with(['Z', 'z']) {
            local_wall_clock(t.timestamp())
        } else {
            t.naive_local().and_utc().timestamp()
        }
    } else if let Ok(t) = DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%z") {
        // Apple writes offsets without a colon ("+0200").
        t.naive_local().and_utc().timestamp()
    } else {
        let n = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
            .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f"))
            .ok()?;
        local_wall_clock(n.and_utc().timestamp())
    };
    plausible(wall).then_some(wall)
}

type Tags = Vec<(String, String)>;

fn tag(tags: &Tags, name: &str) -> Option<String> {
    tags.iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// The capture time found in a video's tags (if any) and its duration (if known).
fn read_video(path: &Path) -> (Option<Capture>, Option<f64>) {
    let Ok(ictx) = video::open_input(path) else {
        return (None, None);
    };
    let fmt: Tags = ictx
        .metadata()
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let best = ictx.streams().best(ffmpeg_next::media::Type::Video);
    let duration = best
        .as_ref()
        .map(|s| video::header_duration(&ictx, s))
        .filter(|d| d.is_finite() && *d > 0.0);
    let stream: Tags = best
        .map(|s| {
            s.metadata()
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect()
        })
        .unwrap_or_default();
    // Apple's tag carries the zone the clip was filmed in; the generic ones are UTC.
    let candidates = [
        tag(&fmt, "com.apple.quicktime.creationdate"),
        tag(&fmt, "creation_time"),
        tag(&stream, "creation_time"),
    ];
    let Some(taken_at) = candidates
        .iter()
        .flatten()
        .find_map(|s| parse_video_time(s))
    else {
        return (None, duration);
    };
    let camera = camera_name(
        tag(&fmt, "com.apple.quicktime.make").as_deref(),
        tag(&fmt, "com.apple.quicktime.model").as_deref(),
    );
    (
        Some(Capture {
            taken_at,
            source: DateSource::Video,
            camera,
            duration,
        }),
        duration,
    )
}

#[cfg(test)]
mod tests {
    #![expect(
        clippy::cast_possible_truncation,
        reason = "hand-built TIFF structures with a few bytes of payload"
    )]
    use super::*;
    use crate::media::video::tests::make_clip;
    use image::ImageEncoder;
    use std::io::Write;

    fn ymd(y: i32, m: u32, d: u32, h: u32, mi: u32, s: u32) -> i64 {
        chrono::NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, mi, s)
            .unwrap()
            .and_utc()
            .timestamp()
    }

    /// A JPEG whose EXIF has the given ASCII tags in IFD0 and in the Exif sub-IFD.
    fn jpeg_with_exif(ifd0: &[(u16, &str)], sub: &[(u16, &str)]) -> Vec<u8> {
        let mut e0: Vec<(u16, u16, u32, Vec<u8>)> = ifd0
            .iter()
            .map(|&(t, s)| (t, 2, s.len() as u32 + 1, [s.as_bytes(), &[0]].concat()))
            .collect();
        let mut e1: Vec<(u16, u16, u32, Vec<u8>)> = sub
            .iter()
            .map(|&(t, s)| (t, 2, s.len() as u32 + 1, [s.as_bytes(), &[0]].concat()))
            .collect();
        if !sub.is_empty() {
            e0.push((0x8769, 4, 1, Vec::new())); // pointer, patched below
        }
        e0.sort_by_key(|e| e.0);
        e1.sort_by_key(|e| e.0);
        let ifd0_len = 2 + e0.len() * 12 + 4;
        let sub_len = if e1.is_empty() {
            0
        } else {
            2 + e1.len() * 12 + 4
        };
        let sub_off = 8 + ifd0_len;
        let mut data_off = (8 + ifd0_len + sub_len) as u32;
        let mut data: Vec<u8> = Vec::new();
        let mut write_ifd = |entries: &[(u16, u16, u32, Vec<u8>)], out: &mut Vec<u8>| {
            out.extend((entries.len() as u16).to_be_bytes());
            for (tag, ty, count, val) in entries {
                out.extend(tag.to_be_bytes());
                out.extend(ty.to_be_bytes());
                out.extend(count.to_be_bytes());
                if *tag == 0x8769 {
                    out.extend((sub_off as u32).to_be_bytes());
                } else if val.len() <= 4 {
                    let mut v = val.clone();
                    v.resize(4, 0);
                    out.extend(v);
                } else {
                    out.extend(data_off.to_be_bytes());
                    data_off += val.len() as u32;
                    data.extend(val);
                }
            }
            out.extend(0u32.to_be_bytes());
        };
        let mut tiff = b"MM\0\x2a\0\0\0\x08".to_vec();
        write_ifd(&e0, &mut tiff);
        if !e1.is_empty() {
            write_ifd(&e1, &mut tiff);
        }
        tiff.extend(data);
        let mut app1 = vec![0xFF, 0xE1];
        app1.extend(((2 + 6 + tiff.len()) as u16).to_be_bytes());
        app1.extend(b"Exif\0\0");
        app1.extend(tiff);

        let img = image::RgbImage::from_pixel(8, 8, image::Rgb([9, 9, 9]));
        let mut jpg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpg)
            .write_image(img.as_raw(), 8, 8, image::ExtendedColorType::Rgb8)
            .unwrap();
        let mut out = jpg[..2].to_vec();
        out.extend(app1);
        out.extend(&jpg[2..]);
        out
    }

    fn capture_of(bytes: &[u8], mtime: i64) -> Capture {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.jpg");
        std::fs::write(&p, bytes).unwrap();
        read_capture(&p, MediaKind::Image, mtime).unwrap()
    }

    #[test]
    fn exif_original_wins_over_other_dates() {
        let jpg = jpeg_with_exif(
            &[
                (0x010F, "Canon"),
                (0x0110, "Canon EOS 5D"),
                (0x0132, "2020:01:01 00:00:00"),
            ],
            &[(0x9003, "2018:05:06 07:08:09")],
        );
        let c = capture_of(&jpg, 1_000_000_000);
        assert_eq!(c.source, DateSource::Exif);
        assert_eq!(c.taken_at, ymd(2018, 5, 6, 7, 8, 9));
        assert_eq!(c.camera.as_deref(), Some("Canon EOS 5D"));
    }

    #[test]
    fn exif_datetime_is_the_fallback_and_camera_is_joined() {
        let jpg = jpeg_with_exif(
            &[
                (0x010F, "Acme"),
                (0x0110, "Phone 2"),
                (0x0132, "2020:02:03 04:05:06"),
            ],
            &[],
        );
        let c = capture_of(&jpg, 1_000_000_000);
        assert_eq!(c.source, DateSource::Exif);
        assert_eq!(c.taken_at, ymd(2020, 2, 3, 4, 5, 6));
        assert_eq!(c.camera.as_deref(), Some("Acme Phone 2"));
    }

    #[test]
    fn implausible_exif_dates_are_skipped() {
        let jpg = jpeg_with_exif(
            &[(0x0132, "2021:03:04 05:06:07")],
            &[(0x9003, "0000:00:00 00:00:00")],
        );
        assert_eq!(
            capture_of(&jpg, 5).taken_at,
            ymd(2021, 3, 4, 5, 6, 7),
            "falls through to DateTime"
        );
        let only_bad = jpeg_with_exif(&[], &[(0x9003, "1960:01:01 00:00:00")]);
        let c = capture_of(&only_bad, 1_500_000_000);
        assert_eq!(c.source, DateSource::File);
        let future = jpeg_with_exif(&[], &[(0x9003, "2999:01:01 00:00:00")]);
        assert_eq!(capture_of(&future, 7).source, DateSource::File);
    }

    #[test]
    fn png_without_exif_uses_the_file_time() {
        let img = image::RgbImage::from_pixel(4, 4, image::Rgb([1, 2, 3]));
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(img.as_raw(), 4, 4, image::ExtendedColorType::Rgb8)
            .unwrap();
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.png");
        std::fs::write(&p, png).unwrap();
        let c = read_capture(&p, MediaKind::Image, 1_600_000_000).unwrap();
        assert_eq!(c.source, DateSource::File);
        assert_eq!(c.taken_at, local_wall_clock(1_600_000_000));
        assert_eq!(c.camera, None);
    }

    #[test]
    fn garbage_and_missing_files_fall_back() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("g.jpg");
        std::fs::write(&p, b"definitely not a jpeg").unwrap();
        for kind in [MediaKind::Image, MediaKind::Video] {
            let c = read_capture(&p, kind, 1_600_000_000).unwrap();
            assert_eq!(c.source, DateSource::File);
        }
    }

    #[test]
    fn unreadable_files_have_no_date_yet() {
        let d = tempfile::tempdir().unwrap();
        for kind in [MediaKind::Image, MediaKind::Video] {
            assert_eq!(
                read_capture(&d.path().join("missing"), kind, 1_600_000_000),
                None,
                "a file that cannot be opened is retried, not dated by a guess"
            );
        }
        // A directory opens but cannot be read: also an I/O failure.
        assert_eq!(
            read_capture(d.path(), MediaKind::Image, 1_600_000_000),
            None
        );
    }

    #[test]
    fn huge_tiff_is_not_parsed() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("big.tif");
        let f = File::create(&p).unwrap();
        // Sparse file: little-endian TIFF header, then a hole past the size limit.
        (&f).write_all(b"II*\0\x08\0\0\0").unwrap();
        f.set_len(MAX_TIFF_EXIF_BYTES + 1).unwrap();
        drop(f);
        let c = read_capture(&p, MediaKind::Image, 1_600_000_000).unwrap();
        assert_eq!(c.source, DateSource::File);
    }

    #[test]
    fn video_creation_time_is_converted_to_local_wall_clock() {
        let d = tempfile::tempdir().unwrap();
        let p = make_clip(
            d.path(),
            "dated.mp4",
            1,
            10,
            &["-metadata", "creation_time=2019-07-04T12:00:00Z"],
        );
        let c = read_capture(&p, MediaKind::Video, 1_000_000_000).unwrap();
        assert_eq!(c.source, DateSource::Video);
        assert!((c.duration.unwrap() - 1.0).abs() < 0.2, "{:?}", c.duration);
        let utc = ymd(2019, 7, 4, 12, 0, 0);
        assert_eq!(c.taken_at, local_wall_clock(utc));
    }

    #[test]
    fn video_time_formats() {
        // An explicit offset keeps its own wall-clock reading.
        assert_eq!(
            parse_video_time("2019-07-04T14:30:00+0200"),
            Some(ymd(2019, 7, 4, 14, 30, 0))
        );
        assert_eq!(
            parse_video_time("2019-07-04T14:30:00+02:00"),
            Some(ymd(2019, 7, 4, 14, 30, 0))
        );
        assert_eq!(
            parse_video_time("2019-07-04 12:00:00"),
            Some(local_wall_clock(ymd(2019, 7, 4, 12, 0, 0)))
        );
        assert_eq!(parse_video_time("1904-01-01T00:00:00.000000Z"), None);
        assert_eq!(parse_video_time("nonsense"), None);
    }

    #[test]
    fn camera_strings_are_bounded() {
        let long = "x".repeat(10_000);
        let other = "y".repeat(10_000);
        let name = camera_name(Some(&long), Some(&other)).unwrap();
        assert_eq!(name.chars().count(), 2 * MAX_CAMERA_CHARS + 1);
        assert_eq!(
            camera_name(None, Some(&long)).unwrap().len(),
            MAX_CAMERA_CHARS
        );
        // Characters, not bytes: no cut inside a multi-byte character.
        let wide = "é".repeat(500);
        assert_eq!(
            camera_name(Some(&wide), None).unwrap().chars().count(),
            MAX_CAMERA_CHARS
        );
    }

    #[test]
    fn camera_names() {
        assert_eq!(
            camera_name(Some("Canon"), Some("canon EOS")).unwrap(),
            "canon EOS"
        );
        assert_eq!(camera_name(None, Some("X1")).unwrap(), "X1");
        assert_eq!(camera_name(None, None), None);
    }
}
