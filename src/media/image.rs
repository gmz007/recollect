//! Still-image decoding and resizing (see `docs/design.md`).
//!
//! JPEGs are decoded with DCT-domain downscaling (`jpeg-decoder`), which is several times faster
//! than a full decode for large photos. Everything else goes through the `image` crate.
//! EXIF orientation is always applied.

use anyhow::{Context, Result, anyhow, bail};
use fast_image_resize as fr;
use image::metadata::Orientation;
use image::{DynamicImage, RgbImage};
use std::path::Path;

/// Refuse to decode absurdly large images (decompression bombs), in pixels.
const MAX_PIXELS: u64 = 250_000_000;
/// Still images larger than this are not decoded (several decode threads run at once).
const MAX_FILE_BYTES: u64 = 512 << 20;

/// A decoded image: packed RGB8.
#[derive(Debug, Clone)]
pub struct Rgb {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// Result of decoding: pixels (possibly downscaled) and the original dimensions
/// (after orientation).
#[derive(Debug, Clone)]
pub struct Decoded {
    pub rgb: Rgb,
    pub orig_width: u32,
    pub orig_height: u32,
}

/// Decodes `path` so that its shorter side is at least `min_side` pixels where the source
/// allows it (smaller sources are returned at full size).
pub fn decode(path: &Path, min_side: u32) -> Result<Decoded> {
    let len = std::fs::metadata(path)
        .with_context(|| format!("reading {}", path.display()))?
        .len();
    if len > MAX_FILE_BYTES {
        bail!("file too large to index ({} MB)", len >> 20);
    }
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    decode_bytes(&bytes, min_side)
}

pub fn decode_bytes(bytes: &[u8], min_side: u32) -> Result<Decoded> {
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        match decode_jpeg_scaled(bytes, min_side) {
            Ok(Some(d)) => return Ok(d),
            Ok(None) => {} // unusual pixel format: let `image` handle it
            Err(e) => tracing::debug!("jpeg-decoder failed ({e}); falling back to image crate"),
        }
    }
    decode_generic(bytes)
}

fn decode_jpeg_scaled(bytes: &[u8], min_side: u32) -> Result<Option<Decoded>> {
    let mut dec = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    dec.read_info()?;
    let info = dec.info().ok_or_else(|| anyhow!("no JPEG header"))?;
    let (w, h) = (u32::from(info.width), u32::from(info.height));
    check_size(w, h)?;
    let fmt = info.pixel_format;
    if !matches!(
        fmt,
        jpeg_decoder::PixelFormat::RGB24 | jpeg_decoder::PixelFormat::L8
    ) {
        return Ok(None);
    }
    let orientation = dec
        .exif_data()
        .and_then(exif_orientation)
        .unwrap_or(Orientation::NoTransforms);
    // Ask for the smallest IDCT scale whose output still covers `min_side` on both axes.
    // `scale` picks the smallest factor satisfying *either* axis, so constrain only the shorter
    // axis (the other request can never be met).
    let want = min_side.min(w).min(h).min(u32::from(u16::MAX)) as u16;
    let (sw, sh) = if w <= h {
        dec.scale(want, u16::MAX)?
    } else {
        dec.scale(u16::MAX, want)?
    };
    let px = dec.decode()?;
    let (sw, sh) = (u32::from(sw), u32::from(sh));
    let data = match fmt {
        jpeg_decoder::PixelFormat::L8 => px.iter().flat_map(|&g| [g, g, g]).collect(),
        _ => px,
    };
    if data.len() != (sw * sh * 3) as usize {
        bail!("decoded JPEG has unexpected size");
    }
    let img = RgbImage::from_raw(sw, sh, data).ok_or_else(|| anyhow!("bad JPEG buffer"))?;
    let mut dynimg = DynamicImage::ImageRgb8(img);
    dynimg.apply_orientation(orientation);
    let (ow, oh) = oriented_dims(w, h, orientation);
    Ok(Some(Decoded {
        rgb: into_rgb(dynimg),
        orig_width: ow,
        orig_height: oh,
    }))
}

fn decode_generic(bytes: &[u8]) -> Result<Decoded> {
    use image::ImageDecoder;
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut dec = reader.into_decoder()?;
    let (w, h) = dec.dimensions();
    check_size(w, h)?;
    let orientation = dec.orientation().unwrap_or(Orientation::NoTransforms);
    let mut img = DynamicImage::from_decoder(dec)?;
    img.apply_orientation(orientation);
    let rgb = into_rgb(img);
    Ok(Decoded {
        orig_width: rgb.width,
        orig_height: rgb.height,
        rgb,
    })
}

fn check_size(w: u32, h: u32) -> Result<()> {
    if w == 0 || h == 0 {
        bail!("image has zero size");
    }
    if u64::from(w) * u64::from(h) > MAX_PIXELS {
        bail!("image too large ({w}×{h})");
    }
    Ok(())
}

fn exif_orientation(raw: &[u8]) -> Option<Orientation> {
    let ex = exif::Reader::new().read_raw(raw.to_vec()).ok()?;
    let f = ex.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
    let v = f.value.get_uint(0)?;
    Orientation::from_exif(u8::try_from(v).ok()?)
}

fn oriented_dims(w: u32, h: u32, o: Orientation) -> (u32, u32) {
    match o {
        Orientation::Rotate90
        | Orientation::Rotate270
        | Orientation::Rotate90FlipH
        | Orientation::Rotate270FlipH => (h, w),
        _ => (w, h),
    }
}

fn into_rgb(img: DynamicImage) -> Rgb {
    let rgb = img.into_rgb8();
    Rgb {
        width: rgb.width(),
        height: rgb.height(),
        data: rgb.into_raw(),
    }
}

/// How to map an image onto the square model input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Scale the short side to the target and center-crop (CLIP).
    CenterCrop,
    /// Squash to the target size ignoring aspect ratio (SigLIP).
    Stretch,
}

/// Resizes to `size×size` RGB8 for the model.
pub fn to_square(img: &Rgb, size: u32, fit: Fit) -> Result<Vec<u8>> {
    let src = fr::images::ImageRef::new(img.width, img.height, &img.data, fr::PixelType::U8x3)?;
    let mut dst = fr::images::Image::new(size, size, fr::PixelType::U8x3);
    let mut opts =
        fr::ResizeOptions::new().resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::CatmullRom));
    if fit == Fit::CenterCrop {
        opts = opts.fit_into_destination(Some((0.5, 0.5)));
    }
    fr::Resizer::new().resize(&src, &mut dst, &opts)?;
    Ok(dst.into_vec())
}

/// Downscales to fit within `max_side`×`max_side`, keeping the aspect ratio (never upscales).
pub fn fit_within(img: &Rgb, max_side: u32) -> Result<Rgb> {
    let scale = (max_side as f32 / img.width.max(img.height) as f32).min(1.0);
    let w = ((img.width as f32 * scale).round() as u32).max(1);
    let h = ((img.height as f32 * scale).round() as u32).max(1);
    if (w, h) == (img.width, img.height) {
        return Ok(img.clone());
    }
    let src = fr::images::ImageRef::new(img.width, img.height, &img.data, fr::PixelType::U8x3)?;
    let mut dst = fr::images::Image::new(w, h, fr::PixelType::U8x3);
    let opts =
        fr::ResizeOptions::new().resize_alg(fr::ResizeAlg::Convolution(fr::FilterType::CatmullRom));
    fr::Resizer::new().resize(&src, &mut dst, &opts)?;
    Ok(Rgb {
        width: w,
        height: h,
        data: dst.into_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageEncoder, Rgb as Px};

    /// Minimal EXIF (TIFF big-endian) with a single Orientation tag.
    fn exif_app1(orientation: u16) -> Vec<u8> {
        let mut tiff = b"MM\0\x2a\0\0\0\x08".to_vec();
        tiff.extend_from_slice(&1u16.to_be_bytes()); // one entry
        tiff.extend_from_slice(&0x0112u16.to_be_bytes()); // Orientation
        tiff.extend_from_slice(&3u16.to_be_bytes()); // SHORT
        tiff.extend_from_slice(&1u32.to_be_bytes()); // count
        tiff.extend_from_slice(&orientation.to_be_bytes());
        tiff.extend_from_slice(&[0, 0]);
        tiff.extend_from_slice(&0u32.to_be_bytes()); // no next IFD
        let mut seg = vec![0xFF, 0xE1];
        let payload_len = 2 + 6 + tiff.len();
        seg.extend_from_slice(&(payload_len as u16).to_be_bytes());
        seg.extend_from_slice(b"Exif\0\0");
        seg.extend_from_slice(&tiff);
        seg
    }

    fn jpeg_bytes(w: u32, h: u32) -> Vec<u8> {
        let img = RgbImage::from_fn(w, h, |x, _| {
            if x < w / 2 {
                Px([255, 0, 0])
            } else {
                Px([0, 0, 255])
            }
        });
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 95)
            .write_image(img.as_raw(), w, h, image::ExtendedColorType::Rgb8)
            .unwrap();
        out
    }

    #[test]
    fn jpeg_is_downscaled_but_covers_min_side() {
        let d = decode_bytes(&jpeg_bytes(1600, 1200), 224).unwrap();
        assert_eq!((d.orig_width, d.orig_height), (1600, 1200));
        assert!(
            d.rgb.height >= 224 && d.rgb.height <= 448,
            "{}",
            d.rgb.height
        );
        assert_eq!(d.rgb.data.len(), (d.rgb.width * d.rgb.height * 3) as usize);
    }

    #[test]
    fn small_images_are_not_upscaled() {
        let d = decode_bytes(&jpeg_bytes(100, 80), 224).unwrap();
        assert_eq!((d.rgb.width, d.rgb.height), (100, 80));
    }

    #[test]
    fn exif_rotation_is_applied() {
        let jpg = jpeg_bytes(64, 32);
        // Insert APP1 right after SOI.
        let mut with_exif = jpg[..2].to_vec();
        with_exif.extend(exif_app1(6)); // rotate 90° clockwise
        with_exif.extend_from_slice(&jpg[2..]);
        let d = decode_bytes(&with_exif, 1000).unwrap();
        assert_eq!((d.orig_width, d.orig_height), (32, 64));
        assert_eq!((d.rgb.width, d.rgb.height), (32, 64));
        // Left half red rotated 90° CW ends up at the top.
        let top = &d.rgb.data[0..3];
        assert!(top[0] > 200 && top[2] < 60, "{top:?}");
    }

    #[test]
    fn png_and_garbage() {
        let img = RgbImage::from_fn(30, 20, |_, _| Px([0, 255, 0]));
        let mut png = Vec::new();
        image::codecs::png::PngEncoder::new(&mut png)
            .write_image(img.as_raw(), 30, 20, image::ExtendedColorType::Rgb8)
            .unwrap();
        let d = decode_bytes(&png, 224).unwrap();
        assert_eq!((d.rgb.width, d.rgb.height), (30, 20));
        assert!(decode_bytes(b"not an image", 224).is_err());
        assert!(decode_bytes(&[0xFF, 0xD8, 0xFF, 0, 1, 2], 224).is_err());
    }

    #[test]
    fn square_and_fit() {
        let d = decode_bytes(&jpeg_bytes(400, 200), 224).unwrap();
        let sq = to_square(&d.rgb, 224, Fit::CenterCrop).unwrap();
        assert_eq!(sq.len(), 224 * 224 * 3);
        // Center crop of a red|blue split: left edge red, right edge blue.
        assert!(sq[0] > 200);
        assert!(sq[(223 * 3) + 2] > 200);
        let st = to_square(&d.rgb, 224, Fit::Stretch).unwrap();
        assert_eq!(st.len(), 224 * 224 * 3);
        let t = fit_within(&d.rgb, 100).unwrap();
        assert_eq!((t.width, t.height), (100, 50));
    }
}
