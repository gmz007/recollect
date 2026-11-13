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
