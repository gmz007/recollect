use super::convert::{frame_sizes, rotate, rotation_from_matrix};
use super::stream::{chain_is_io_failure, finite_positive, is_io_failure};
use super::stream::{checked_time_base, stream_time};
use super::*;
use anyhow::anyhow;
use std::path::Path;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn square_pixels_scale_the_shorter_side_down_to_the_limit() {
    let (display, out) = frame_sizes(1920, 1080, 1.0, 270);
    assert_eq!(display, (1920, 1080));
    assert_eq!(out, (480, 270));
}

#[test]
fn anamorphic_pixels_widen_the_display_size() {
    let (display, out) = frame_sizes(720, 576, 64.0 / 45.0, 288);
    assert_eq!(display, (1024, 576));
    assert_eq!(out, (512, 288));
}

#[test]
fn an_absurd_pixel_aspect_ratio_falls_back_to_square_pixels() {
    for sar in [100_000.0, 1.0 / 100_000.0, f64::NAN, f64::INFINITY] {
        let (display, out) = frame_sizes(1920, 1080, sar, 270);
        assert_eq!(display, (1920, 1080), "sar {sar}");
        assert_eq!(out, (480, 270), "sar {sar}");
    }
}

#[test]
fn a_vanished_drive_counts_as_an_io_failure_but_damaged_data_does_not() {
    let io = ff::Error::Other {
        errno: ff::error::EIO,
    };
    let wrapped = anyhow::Error::new(io).context("reading packet");
    assert!(is_io_failure(io));
    assert!(chain_is_io_failure(&wrapped));
    assert!(!is_io_failure(ff::Error::InvalidData));
    assert!(!chain_is_io_failure(&anyhow::Error::new(
        ff::Error::InvalidData
    )));
    assert!(!chain_is_io_failure(&anyhow!("plain")));
}

/// Generates a test clip with ffmpeg. Left half red / right half blue with a moving bar,
/// `secs` long at 10 fps, keyframe every `gop` frames. Extra args go before the output.
pub fn make_clip(dir: &Path, name: &str, secs: u32, gop: u32, extra: &[&str]) -> PathBuf {
    let out = dir.join(name);
    let src = format!(
        "color=c=red:s=160x90:r=10:d={secs}[a];color=c=blue:s=160x90:r=10:d={secs}[b];\
         [a][b]hstack,drawbox=x='mod(t*40,300)':y=0:w=10:h=90:c=white:t=fill"
    );
    let status = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-y", "-filter_complex", &src])
        .args([
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-g",
            &gop.to_string(),
            "-keyint_min",
            &gop.to_string(),
        ])
        .args(["-sc_threshold", "0"])
        .args(extra)
        .arg(&out)
        .status()
        .expect("ffmpeg must be installed to run video tests");
    assert!(status.success());
    out
}

fn sample_all(p: &Path, cfg: SamplerConfig) -> (Vec<f64>, VideoInfo) {
    let mut ts = Vec::new();
    let info = sample(p, cfg, 64, &|| false, &mut |f| {
        assert!(f.rgb.width.min(f.rgb.height) <= 90);
        ts.push(f.t);
        Ok(())
    })
    .unwrap();
    (ts, info)
}

#[test]
fn hostile_header_values_are_contained() {
    // Time bases must be finite and positive.
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(checked_time_base(bad).is_err(), "{bad}");
    }
    assert_eq!(checked_time_base(1.0 / 90_000.0).unwrap(), 1.0 / 90_000.0);
    // Durations that are not finite and positive count as unknown.
    for bad in [0.0, -3.0, f64::NAN, f64::INFINITY] {
        assert_eq!(finite_positive(bad), 0.0, "{bad}");
    }
    assert_eq!(finite_positive(12.5), 12.5);
    // Extreme timestamps and start times do not overflow.
    assert!(stream_time(i64::MIN, i64::MAX, 1e-5, 0.0).is_finite());
    assert!(stream_time(i64::MAX, i64::MIN, 1e-5, 0.0).is_finite());
    assert_eq!(stream_time(100, 40, 0.5, 1.0), 31.0);
}

#[test]
fn a_frame_buffer_that_does_not_match_its_size_is_an_error() {
    let ok = Rgb {
        width: 2,
        height: 1,
        data: vec![0; 6],
    };
    let turned = rotate(ok, 90).unwrap();
    assert_eq!((turned.width, turned.height), (1, 2));
    let short = Rgb {
        width: 2,
        height: 2,
        data: vec![0; 5],
    };
    assert!(rotate(short, 90).is_err());
}

#[test]
fn short_gop_uses_keyframes_at_interval() {
    let d = tempfile::tempdir().unwrap();
    let p = make_clip(d.path(), "short.mp4", 20, 10, &[]); // keyframe every 1 s
    let (ts, info) = sample_all(&p, SamplerConfig::default());
    assert!((info.duration - 20.0).abs() < 0.5, "{}", info.duration);
    assert_eq!((info.width, info.height), (320, 90), "{info:?}");
    assert!(ts.len() >= 9 && ts.len() <= 11, "{ts:?}");
    assert!(
        ts.windows(2)
            .all(|w| w[1] - w[0] >= 1.9 && w[1] - w[0] <= 4.01),
        "{ts:?}"
    );
}

#[test]
fn long_gop_still_samples_every_interval() {
    let d = tempfile::tempdir().unwrap();
    let p = make_clip(d.path(), "long.mp4", 20, 200, &[]); // one keyframe only
    let (ts, _) = sample_all(&p, SamplerConfig::default());
    assert!(ts.len() >= 9 && ts.len() <= 11, "{ts:?}");
    assert!(
        ts.windows(2).all(|w| (w[1] - w[0] - 2.0).abs() < 0.15),
        "{ts:?}"
    );
    assert!(ts[0] < 0.2);
}

#[test]
fn frame_at_and_rotation_match_ffmpeg() {
    let d = tempfile::tempdir().unwrap();
    let p = make_clip(d.path(), "rot.mp4", 4, 10, &["-metadata:s:v", "rotate=0"]);
    let rot = d.path().join("rot90.mp4");
    let st = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-y", "-display_rotation", "90", "-i"])
        .arg(&p)
        .args(["-c", "copy"])
        .arg(&rot)
        .status()
        .unwrap();
    assert!(st.success());
    let plain = frame_at(&p, 1.0, 90).unwrap();
    assert_eq!((plain.width, plain.height), (320, 90));
    let r = frame_at(&rot, 1.0, 90).unwrap();
    assert_eq!((r.width, r.height), (90, 320), "rotated display");
    // Compare with ffmpeg's own autorotation: which end of the tall frame is red?
    let png = d.path().join("ref.png");
    let st = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-y", "-ss", "1", "-i"])
        .arg(&rot)
        .args(["-frames:v", "1"])
        .arg(&png)
        .status()
        .unwrap();
    assert!(st.success());
    let reference = image::open(&png).unwrap().to_rgb8();
    assert_eq!(reference.dimensions(), (90, 320));
    let ref_top_red = reference.get_pixel(45, 5)[0] > 128;
    let top_red = r.data[(5 * 90 + 45) * 3] > 128;
    assert_eq!(top_red, ref_top_red);
    let (_, info) = sample_all(&rot, SamplerConfig::default());
    assert_eq!((info.width, info.height), (90, 320));
}

#[test]
fn oversized_gop_streams_without_losing_frames() {
    let d = tempfile::tempdir().unwrap();
    let p = make_clip(d.path(), "big.mp4", 20, 200, &[]);
    let cfg = SamplerConfig {
        max_gop_bytes: 2_000,
        ..Default::default()
    };
    let (ts, _) = sample_all(&p, cfg);
    assert!(ts.len() >= 9 && ts.len() <= 11, "{ts:?}");
    assert!(
        ts.windows(2).all(|w| (w[1] - w[0] - 2.0).abs() < 0.15),
        "{ts:?}"
    );
}

#[test]
fn single_gop_without_known_duration() {
    let d = tempfile::tempdir().unwrap();
    let p = make_clip(d.path(), "src.mp4", 20, 200, &[]);
    // Matroska written to a pipe has no duration and no cues.
    let out = d.path().join("nodur.mkv");
    let st = Command::new("sh")
        .arg("-c")
        .arg(format!(
            "ffmpeg -loglevel error -i '{}' -c copy -f matroska - > '{}'",
            p.display(),
            out.display()
        ))
        .status()
        .unwrap();
    assert!(st.success());
    let (ts, info) = sample_all(&out, SamplerConfig::default());
    assert!(ts.len() >= 9, "{ts:?}");
    assert!(info.duration >= 18.0, "{info:?}");
}

#[test]
fn frame_past_the_end_returns_the_last_frame() {
    let d = tempfile::tempdir().unwrap();
    let p = make_clip(d.path(), "e.mp4", 4, 10, &[]);
    let f = frame_at(&p, 100.0, 90).unwrap();
    assert_eq!((f.width, f.height), (320, 90));
}

#[test]
fn concatenated_timestamps_keep_increasing() {
    let d = tempfile::tempdir().unwrap();
    let a = make_clip(d.path(), "a.ts", 10, 10, &["-f", "mpegts"]);
    // Byte-level concatenation: timestamps restart in the middle of the file.
    let joined = d.path().join("joined.ts");
    let mut bytes = std::fs::read(&a).unwrap();
    bytes.extend(std::fs::read(&a).unwrap());
    std::fs::write(&joined, bytes).unwrap();
    let (ts, _) = sample_all(&joined, SamplerConfig::default());
    assert!(ts.len() >= 8, "only {} samples: {ts:?}", ts.len());
    assert!(ts.windows(2).all(|w| w[1] > w[0]), "{ts:?}");
}

#[test]
fn rotation_matrix_math() {
    // Identity.
    assert_eq!(
        rotation_from_matrix(&[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0]),
        0
    );
    // 180°.
    assert_eq!(
        rotation_from_matrix(&[-1.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 1.0]),
        180
    );
    assert_eq!(rotation_from_matrix(&[0.0; 9]), 0);
}

#[test]
fn garbage_is_an_error() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("x.mp4");
    std::fs::write(&p, b"definitely not a video").unwrap();
    assert!(sample(&p, SamplerConfig::default(), 64, &|| false, &mut |_| Ok(())).is_err());
    assert!(frame_at(&p, 0.0, 64).is_err());
}

#[test]
fn cancel_stops_sampling() {
    let d = tempfile::tempdir().unwrap();
    let p = make_clip(d.path(), "c.mp4", 10, 10, &[]);
    let err = sample(&p, SamplerConfig::default(), 64, &|| true, &mut |_| Ok(())).unwrap_err();
    assert!(err.to_string().contains("cancelled"));
}

#[test]
fn storage_is_reachable_for_an_existing_file_and_not_for_a_missing_one() {
    let dir = tempfile::tempdir().unwrap();
    let small = dir.path().join("small.bin");
    std::fs::write(&small, b"tiny").unwrap();
    let empty = dir.path().join("empty.bin");
    std::fs::write(&empty, b"").unwrap();
    let big = dir.path().join("big.bin");
    std::fs::write(&big, vec![7u8; 100_000]).unwrap();
    assert!(!super::stream::storage_unreachable(&small));
    assert!(!super::stream::storage_unreachable(&empty));
    assert!(!super::stream::storage_unreachable(&big));
    assert!(super::stream::storage_unreachable(
        &dir.path().join("gone.bin")
    ));
}
