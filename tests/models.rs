//! Tests with the real models. Ignored by default; run with
//! `RECOLLECT_TEST_MODELS=<dir with clip-vit-b32/ and/or siglip-base-16/> cargo nextest run --run-ignored all`.

use recollect::model::{Device, VisionEncoder, registry};
use std::path::PathBuf;

fn model_dir(id: &str) -> Option<PathBuf> {
    let d = PathBuf::from(std::env::var_os("RECOLLECT_TEST_MODELS")?).join(id);
    d.is_dir().then_some(d)
}

fn images(n: usize, size: u32) -> Vec<Vec<u8>> {
    (0..n)
        .map(|i| {
            (0..size * size * 3)
                .map(|p| ((p as usize * (i + 3)) % 251) as u8)
                .collect()
        })
        .collect()
}

fn cos(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[test]
#[ignore = "needs downloaded models (RECOLLECT_TEST_MODELS)"]
fn padding_and_devices_give_the_same_embeddings() {
    for spec in registry::MODELS {
        let Some(dir) = model_dir(spec.id) else {
            continue;
        };
        let imgs = images(3, spec.image_size);
        let refs: Vec<&[u8]> = imgs.iter().map(|v| v.as_slice()).collect();
        let mut cpu = VisionEncoder::load(spec, &dir, false, 4).unwrap();
        assert_eq!(cpu.device(), Device::Cpu);
        let batch = cpu.embed(&refs).unwrap();
        for (i, r) in refs.iter().enumerate() {
            let single = cpu.embed(&[r]).unwrap();
            assert!(
                cos(&single[0], &batch[i]) > 0.999,
                "{} batch vs single",
                spec.id
            );
        }
        let mut gpu = VisionEncoder::load(spec, &dir, true, 4).unwrap();
        if gpu.device() == Device::Gpu {
            let g = gpu.embed(&refs).unwrap(); // padded to the GPU batch size
            for i in 0..3 {
                assert!(
                    cos(&g[i], &batch[i]) > 0.995,
                    "{} GPU vs CPU: {}",
                    spec.id,
                    cos(&g[i], &batch[i])
                );
            }
        }
    }
}

/// Copies (hard-links where possible) a test model into a fresh Recollect home.
fn home_with_model(id: &str) -> Option<(tempfile::TempDir, recollect::config::Paths)> {
    let src = model_dir(id)?;
    let home = tempfile::tempdir().unwrap();
    let paths = recollect::config::Paths::under(home.path());
    let dst = paths.models_dir().join(id);
    std::fs::create_dir_all(&dst).unwrap();
    for e in std::fs::read_dir(&src).unwrap().flatten() {
        if !e.path().is_file() {
            continue;
        }
        let to = dst.join(e.file_name());
        if std::fs::hard_link(e.path(), &to).is_err() {
            std::fs::copy(e.path(), &to).unwrap();
        }
    }
    Some((home, paths))
}

#[test]
#[ignore = "needs downloaded models (RECOLLECT_TEST_MODELS)"]
fn cancel_resume_errors_and_search_end_to_end() {
    use recollect::Library;
    use recollect::library::Activity;
    use recollect::search::SearchOptions;
    let Some((spec, (_home, paths))) = registry::MODELS
        .iter()
        .find_map(|m| Some((m, home_with_model(m.id)?)))
    else {
        eprintln!("SKIPPED: no model under RECOLLECT_TEST_MODELS");
        return;
    };
    recollect::gpu::force_cpu(true);
    let media = tempfile::tempdir().unwrap();
    let colors = [
        ("red", [220u8, 20, 20]),
        ("green", [20, 180, 40]),
        ("blue", [30, 40, 220]),
    ];
    for (name, rgb) in colors {
        for i in 0..10u32 {
            let img = image::RgbImage::from_fn(320, 240, |x, y| {
                let n = ((x * 7 + y * 13 + i * 31) % 17) as u8;
                image::Rgb([
                    rgb[0].saturating_add(n),
                    rgb[1].saturating_add(n),
                    rgb[2].saturating_add(n),
                ])
            });
            img.save(media.path().join(format!("{name}-{i}.png")))
                .unwrap();
        }
    }
    std::fs::write(media.path().join("broken.jpg"), b"\xFF\xD8\xFF garbage").unwrap();

    {
        let lib = Library::open(paths.clone(), Some(spec.id)).unwrap();
        lib.add_folder(media.path()).unwrap();
        // Cancel deterministically as soon as the first file is done.
        let weak = std::sync::Arc::downgrade(&lib);
        lib.set_notifier(move || {
            if let Some(l) = weak.upgrade()
                && let Activity::Indexing(p) = l.status().activity
                && p.done > 0
            {
                l.cancel();
            }
        });
        lib.start_indexing().unwrap();
        lib.wait();
        let s = lib.stats().unwrap();
        assert_eq!(s.indexed + s.pending + s.errors, 31, "{s:?}");
        assert!(
            s.pending > 0,
            "cancel should leave work for the resume: {s:?}"
        );
        assert_eq!(lib.indexed_items(), s.items, "index and database agree");
    }
    // "Restart": a new Library on the same data resumes where the last run stopped.
    let lib = Library::open(paths, Some(spec.id)).unwrap();
    let sum = lib.index_blocking().unwrap();
    assert!(!sum.cancelled);
    let s = lib.stats().unwrap();
    assert_eq!((s.indexed, s.pending, s.errors), (30, 0, 1), "{s:?}");
    assert_eq!(lib.indexed_items(), 30);
    let hits = lib
        .search(
            "a plain red picture",
            &SearchOptions {
                limit: 5,
                kind: None,
                ..Default::default()
            },
        )
        .unwrap();
    assert!(!hits.is_empty(), "search returned nothing");
    let top = hits[0]
        .path
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_string();
    assert!(top.starts_with("red"), "top hit {top}");
    // Rescan with nothing changed does no work.
    let again = lib.index_blocking().unwrap();
    assert_eq!(
        (again.files, again.added, again.changed, again.removed),
        (0, 0, 0, 0)
    );
}
