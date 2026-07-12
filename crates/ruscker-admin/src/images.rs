//! Image processing — MIME sniffing, decode, WebP re-encoding.
//!
//! Operators can upload PNG/JPEG/SVG/WebP. PNG and JPEG are
//! re-encoded to WebP at quality 80 (best size/visual trade-off
//! for screenshots and logos). SVG is passed through unchanged
//! since vector logos shouldn't be rasterized; the MIME stays
//! `image/svg+xml`. WebP uploads are passed through as-is.
//!
//! Returned [`Processed`] carries everything `db::images::insert`
//! needs to write a row. No DB code lives here so this module
//! stays isolated and unit-testable. Runtime entry points move the
//! CPU-bound work to a shared, concurrency-limited blocking pool.

use anyhow::{anyhow, Context, Result};
use image::{ImageReader, Limits};
use std::io::Cursor;
use std::sync::{Arc, LazyLock};
use tokio::sync::Semaphore;

/// Maximum source upload size accepted. Anything bigger is
/// rejected before decoding — protects against
/// decompression-bomb-style memory attacks.
pub const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;

/// Maximum width or height accepted for a raster image. A compressed-byte
/// cap alone cannot bound the decoded allocation of an image bomb.
pub const MAX_IMAGE_DIMENSION: u32 = 8192;

/// Maximum decoded pixel count. This independently rejects wide-but-legal
/// dimensions whose product would still require excessive memory.
pub const MAX_IMAGE_PIXELS: u64 = 16_000_000;

/// Decoder allocation budget, below `image`'s generic 512 MiB default.
pub const MAX_DECODE_ALLOC_BYTES: u64 = 64 * 1024 * 1024;

/// Image decoders and WebP encoders are CPU- and memory-heavy. Keep one
/// shared process-wide budget across uploads, imports, and thumbnails.
pub const MAX_CONCURRENT_IMAGE_JOBS: usize = 2;

static IMAGE_JOB_SLOTS: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(MAX_CONCURRENT_IMAGE_JOBS)));

/// Soft target after re-encoding. Larger uploads still go through
/// but produce a `tracing::warn`. Used by callers to surface a UI
/// warning.
pub const SOFT_TARGET_BYTES: usize = 500 * 1024;

/// Output of [`process_upload`] — everything `db::images::insert`
/// needs and nothing it doesn't.
#[derive(Debug)]
pub struct Processed {
    pub filename: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// Run image CPU work away from Tokio workers while bounding the number of
/// simultaneous decodes/transcodes. The owned permit moves into the blocking
/// closure so cancelling the request cannot release capacity while its
/// non-cancellable `spawn_blocking` job is still running.
async fn run_blocking_image_job<T, F>(slots: Arc<Semaphore>, job: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let permit = slots
        .acquire_owned()
        .await
        .map_err(|_| anyhow!("image processor unavailable"))?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        job()
    })
    .await
    .context("image processing task failed")
}

/// Async entry point for HTTP uploads and other runtime callers.
pub async fn process_upload_async(
    raw_filename: String,
    supplied_mime: Option<String>,
    bytes: Vec<u8>,
) -> Result<Processed> {
    run_blocking_image_job(IMAGE_JOB_SLOTS.clone(), move || {
        process_upload(&raw_filename, supplied_mime.as_deref(), bytes)
    })
    .await?
}

/// Async migration-import entry point using the same resource budget.
pub async fn process_for_import_async(
    raw_filename: String,
    bytes: Vec<u8>,
) -> Option<Processed> {
    match run_blocking_image_job(IMAGE_JOB_SLOTS.clone(), move || {
        process_for_import(&raw_filename, bytes)
    })
    .await
    {
        Ok(processed) => processed,
        Err(err) => {
            tracing::warn!(error = ?err, "image import processing task failed");
            None
        }
    }
}

/// Async thumbnail entry point using the same resource budget.
pub async fn thumbnail_webp_async(bytes: Vec<u8>, max_dim: u32) -> Result<Vec<u8>> {
    run_blocking_image_job(IMAGE_JOB_SLOTS.clone(), move || {
        thumbnail_webp(&bytes, max_dim)
    })
    .await?
}

/// Accepted MIME categories.
#[derive(Debug, PartialEq, Eq)]
pub enum SourceKind {
    Png,
    Jpeg,
    Webp,
    Svg,
}

impl SourceKind {
    /// Sniff bytes (preferred) and fall back to the operator-
    /// supplied content-type. Returns `None` for anything outside
    /// the allowlist.
    pub fn detect(bytes: &[u8], supplied_mime: Option<&str>) -> Option<Self> {
        // 1. Magic-byte sniff — fast and lie-proof for the raster
        //    formats. We only act on positive raster matches here;
        //    a "yes that's XML" answer from infer doesn't help us
        //    distinguish SVG from any other XML.
        if let Some(kind) = infer::get(bytes) {
            match kind.mime_type() {
                "image/png" => return Some(Self::Png),
                "image/jpeg" => return Some(Self::Jpeg),
                "image/webp" => return Some(Self::Webp),
                _ => {} // fall through
            }
        }
        // 2. SVG: text-sniff because it's plain XML.
        if looks_like_svg(bytes) {
            return Some(Self::Svg);
        }
        // 3. Last resort: trust the client-declared MIME.
        match supplied_mime? {
            "image/svg+xml" => Some(Self::Svg),
            "image/png" => Some(Self::Png),
            "image/jpeg" => Some(Self::Jpeg),
            "image/webp" => Some(Self::Webp),
            _ => None,
        }
    }
}

fn looks_like_svg(bytes: &[u8]) -> bool {
    let prefix = std::str::from_utf8(&bytes[..bytes.len().min(512)]).unwrap_or("");
    let prefix = prefix.trim_start();
    prefix.starts_with("<?xml") && prefix.contains("<svg")
        || prefix.starts_with("<svg")
}

/// Decode + re-encode the upload. Returns `Err` on unsupported
/// MIME, decompression-bomb-sized inputs, or decode failures.
///
/// Filenames are normalized: any path components stripped, then
/// the extension rewritten to `.webp` for PNG/JPEG conversions.
/// SVG keeps `.svg`. WebP keeps `.webp`.
pub fn process_upload(
    raw_filename: &str,
    supplied_mime: Option<&str>,
    bytes: Vec<u8>,
) -> Result<Processed> {
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(anyhow!(
            "upload exceeds {} MB cap",
            MAX_UPLOAD_BYTES / 1024 / 1024
        ));
    }
    let kind = SourceKind::detect(&bytes, supplied_mime)
        .ok_or_else(|| anyhow!("unsupported MIME (PNG/JPEG/WebP/SVG only)"))?;

    let base = sanitize_basename(raw_filename);
    if base.is_empty() {
        return Err(anyhow!("filename cannot be empty"));
    }

    match kind {
        SourceKind::Png | SourceKind::Jpeg => {
            let (webp_bytes, w, h) = encode_to_webp(&bytes)?;
            let filename = rewrite_extension(&base, "webp");
            if webp_bytes.len() > SOFT_TARGET_BYTES {
                tracing::warn!(
                    filename = %filename,
                    bytes = webp_bytes.len(),
                    "image larger than {} KB after WebP encoding",
                    SOFT_TARGET_BYTES / 1024
                );
            }
            Ok(Processed {
                filename,
                mime_type: "image/webp".into(),
                bytes: webp_bytes,
                width: Some(w),
                height: Some(h),
            })
        }
        SourceKind::Webp => {
            // Pass the original WebP bytes through, but still fully decode
            // once under the product limits so corrupt/truncated files are
            // never committed to the Media library.
            let img = decode_raster(&bytes)?;
            let (w, h) = (img.width(), img.height());
            Ok(Processed {
                filename: rewrite_extension(&base, "webp"),
                mime_type: "image/webp".into(),
                bytes,
                width: Some(w),
                height: Some(h),
            })
        }
        SourceKind::Svg => Ok(Processed {
            filename: rewrite_extension(&base, "svg"),
            mime_type: "image/svg+xml".into(),
            bytes,
            width: None,
            height: None,
        }),
    }
}

/// Build a [`Processed`] for a **migration import**: preserve the
/// ORIGINAL basename (case + extension) and the raw bytes, so a
/// ShinyProxy spec's `/assets/img/Snap_Aurora.png` logo reference keeps
/// resolving once the binary lands in the Media library (the lookup is an
/// exact, case-sensitive filename match).
///
/// This deliberately differs from [`process_upload`], which lowercases
/// the name and transcodes PNG/JPEG to WebP — that renames the file and
/// would break the reference, the opposite of what a migration wants. The
/// operator can re-upload through the admin later to get the WebP
/// optimization. Only the directory components are stripped (no
/// traversal); `None` for anything that doesn't sniff as a supported
/// image, or an oversize file.
pub fn process_for_import(raw_filename: &str, bytes: Vec<u8>) -> Option<Processed> {
    if bytes.is_empty() || bytes.len() > MAX_UPLOAD_BYTES {
        return None;
    }
    let kind = SourceKind::detect(&bytes, None)?;
    // Keep the original basename verbatim — only drop directory parts.
    let filename = std::path::Path::new(raw_filename)
        .file_name()
        .and_then(|s| s.to_str())
        .map(str::to_string)?;
    if filename.is_empty() || filename.starts_with('.') {
        return None;
    }
    let (mime, dims) = match kind {
        SourceKind::Png => ("image/png", Some(decode_raster(&bytes).ok()?)),
        SourceKind::Jpeg => ("image/jpeg", Some(decode_raster(&bytes).ok()?)),
        SourceKind::Webp => ("image/webp", Some(decode_raster(&bytes).ok()?)),
        SourceKind::Svg => ("image/svg+xml", None),
    };
    Some(Processed {
        filename,
        mime_type: mime.into(),
        bytes,
        width: dims.as_ref().map(image::DynamicImage::width),
        height: dims.as_ref().map(image::DynamicImage::height),
    })
}

/// Strip any directory components and any leading dots, lowercase
/// the result. Prevents path-traversal via crafted filenames.
pub(crate) fn sanitize_basename(name: &str) -> String {
    let basename = std::path::Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    basename
        .trim_start_matches('.')
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .collect::<String>()
        .to_ascii_lowercase()
}

pub(crate) fn rewrite_extension(name: &str, new_ext: &str) -> String {
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    format!("{stem}.{new_ext}")
}

fn decode_limits() -> Limits {
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_ALLOC_BYTES);
    limits
}

fn metadata_limits() -> Limits {
    let mut limits = Limits::default();
    // Leave width/height unset here so `validate_dimensions` can return the
    // product-specific limit in the operator-facing error. The allocation
    // budget still constrains any decoder work needed to inspect metadata.
    limits.max_alloc = Some(MAX_DECODE_ALLOC_BYTES);
    limits
}

fn image_reader(bytes: &[u8], limits: Limits) -> Result<ImageReader<Cursor<&[u8]>>> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("guess format")?;
    reader.limits(limits);
    Ok(reader)
}

fn validate_dimensions(width: u32, height: u32) -> Result<u64> {
    if width == 0 || height == 0 {
        return Err(anyhow!("image dimensions must be non-zero"));
    }
    if width > MAX_IMAGE_DIMENSION || height > MAX_IMAGE_DIMENSION {
        return Err(anyhow!(
            "image dimensions {width}x{height} exceed {MAX_IMAGE_DIMENSION}x{MAX_IMAGE_DIMENSION} limit"
        ));
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| anyhow!("image pixel count overflow"))?;
    if pixels > MAX_IMAGE_PIXELS {
        return Err(anyhow!(
            "image has {pixels} pixels; limit is {MAX_IMAGE_PIXELS}"
        ));
    }
    let rgba_bytes = pixels
        .checked_mul(4)
        .ok_or_else(|| anyhow!("image allocation size overflow"))?;
    if rgba_bytes > MAX_DECODE_ALLOC_BYTES {
        return Err(anyhow!(
            "decoded image needs at least {rgba_bytes} bytes; limit is {MAX_DECODE_ALLOC_BYTES}"
        ));
    }
    Ok(pixels)
}

/// Read only decoder metadata first, validate the product with checked
/// arithmetic, then let full decode happen under strict decoder limits.
fn decode_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    let dims = image_reader(bytes, metadata_limits())?
        .into_dimensions()
        .context("read image dimensions")?;
    validate_dimensions(dims.0, dims.1)?;
    Ok(dims)
}

fn decode_raster(bytes: &[u8]) -> Result<image::DynamicImage> {
    let expected = decode_dimensions(bytes)?;
    let img = image_reader(bytes, decode_limits())?
        .decode()
        .context("decode source image")?;
    if (img.width(), img.height()) != expected {
        return Err(anyhow!("image dimensions changed while decoding"));
    }
    Ok(img)
}

fn encode_to_webp(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32)> {
    let img = decode_raster(bytes)?;

    let (w, h) = (img.width(), img.height());
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len() / 2);
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::WebP)
        .context("write WebP")?;
    Ok((out, w, h))
}

/// Decode `bytes`, scale to fit within `max_dim`×`max_dim` (aspect
/// preserved, never upscaled), and re-encode as WebP. Serves small
/// gallery thumbnails so the spec form / media page don't ship the
/// full-size blob per `<img>` (#283). Raster formats only — the caller
/// skips SVG (vector, already tiny).
pub fn thumbnail_webp(bytes: &[u8], max_dim: u32) -> Result<Vec<u8>> {
    let img = decode_raster(bytes)?;
    // `thumbnail` preserves aspect ratio and never upscales, using a
    // fast filter — fine for a small tile.
    let thumb = img.thumbnail(max_dim, max_dim);
    let mut out: Vec<u8> = Vec::new();
    thumb
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::WebP)
        .context("write WebP thumbnail")?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn thumbnail_webp_scales_within_max_dim_preserving_aspect() {
        // 200×100 source → fits within 96×96, aspect kept (96×48).
        let src = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            200,
            100,
            image::Rgba([10, 20, 30, 255]),
        ));
        let mut png = Vec::new();
        src.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let thumb = thumbnail_webp(&png, 96).unwrap();
        let (w, h) = decode_dimensions(&thumb).unwrap();
        assert_eq!((w, h), (96, 48), "scaled to fit, aspect preserved");
        assert!(thumb.len() < 5000, "thumbnail is compact: {} bytes", thumb.len());
    }

    #[test]
    fn sanitize_strips_paths_and_normalizes() {
        assert_eq!(sanitize_basename("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_basename("foo bar.png"), "foobar.png");
        assert_eq!(sanitize_basename("CAPS.PNG"), "caps.png");
        assert_eq!(sanitize_basename(".hidden"), "hidden");
    }

    #[test]
    fn detects_svg_text() {
        let svg = br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"/>"#;
        assert_eq!(SourceKind::detect(svg, None), Some(SourceKind::Svg));
        let bare = br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#;
        assert_eq!(SourceKind::detect(bare, None), Some(SourceKind::Svg));
    }

    #[test]
    fn rejects_unknown_mime() {
        let junk = b"this is not an image at all";
        assert!(SourceKind::detect(junk, None).is_none());
    }

    #[test]
    fn process_rejects_oversize() {
        let bytes = vec![0u8; MAX_UPLOAD_BYTES + 1];
        assert!(process_upload("x.png", Some("image/png"), bytes).is_err());
    }

    fn overwide_png() -> Vec<u8> {
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            MAX_IMAGE_DIMENSION + 1,
            1,
            image::Rgba([1, 2, 3, 255]),
        ));
        let mut png = Vec::new();
        img.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        png
    }

    #[test]
    fn rejects_extreme_dimensions_before_full_decode() {
        // One row keeps the fixture tiny while exceeding the product's
        // strict width budget. The metadata pass rejects it before the
        // decoder allocates an RGBA output buffer.
        let err = process_upload("wide.png", Some("image/png"), overwide_png()).unwrap_err();
        assert!(
            err.to_string().contains("exceed"),
            "operator-facing error should name the limit: {err:#}"
        );
    }

    #[test]
    fn imports_and_thumbnails_apply_the_same_dimension_budget() {
        let png = overwide_png();
        assert!(process_for_import("wide.png", png.clone()).is_none());
        assert!(thumbnail_webp(&png, 96).is_err());
    }

    #[test]
    fn dimension_budget_uses_checked_wide_arithmetic() {
        assert_eq!(validate_dimensions(4_000, 4_000).unwrap(), MAX_IMAGE_PIXELS);
        assert!(validate_dimensions(4_000, 4_001).is_err());
        assert!(validate_dimensions(u32::MAX, u32::MAX).is_err());
        assert!(validate_dimensions(0, 1).is_err());
    }

    fn tiny_png() -> Vec<u8> {
        let img = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([1, 2, 3, 255]),
        ));
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        png
    }

    fn tiny_raster(format: image::ImageFormat) -> Vec<u8> {
        let img = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            3,
            2,
            image::Rgb([1, 2, 3]),
        ));
        let mut bytes = Vec::new();
        img.write_to(&mut Cursor::new(&mut bytes), format).unwrap();
        bytes
    }

    #[test]
    fn accepts_valid_png_jpeg_and_webp_with_dimensions() {
        for (name, mime, format) in [
            ("ok.png", "image/png", image::ImageFormat::Png),
            ("ok.jpg", "image/jpeg", image::ImageFormat::Jpeg),
            ("ok.webp", "image/webp", image::ImageFormat::WebP),
        ] {
            let processed = process_upload(name, Some(mime), tiny_raster(format)).unwrap();
            assert_eq!(processed.mime_type, "image/webp");
            assert_eq!((processed.width, processed.height), (Some(3), Some(2)));
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn blocking_jobs_never_exceed_the_shared_slot_budget() {
        let slots = Arc::new(Semaphore::new(MAX_CONCURRENT_IMAGE_JOBS));
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();

        for _ in 0..6 {
            let slots = slots.clone();
            let active = active.clone();
            let peak = peak.clone();
            tasks.push(tokio::spawn(async move {
                run_blocking_image_job(slots, move || {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(30));
                    active.fetch_sub(1, Ordering::SeqCst);
                })
                .await
                .unwrap();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), MAX_CONCURRENT_IMAGE_JOBS);
    }

    #[test]
    fn process_for_import_preserves_name_and_does_not_transcode() {
        // A ShinyProxy logo ref is `/assets/img/Snap_Aurora.png`; the
        // import must keep the exact name + PNG bytes so the (case-
        // sensitive) Media lookup resolves it. Unlike process_upload,
        // which would lowercase + rewrite to .webp.
        let png = tiny_png();
        let p = super::process_for_import("/etc/sp/assets/img/Snap_Aurora.png", png.clone())
            .expect("png accepted");
        assert_eq!(p.filename, "Snap_Aurora.png", "name + case + ext preserved");
        assert_eq!(p.mime_type, "image/png", "kept as PNG, not transcoded");
        assert_eq!(p.bytes, png, "bytes untouched");
        assert_eq!(p.width, Some(2));
    }

    #[test]
    fn process_for_import_rejects_non_images_and_dotfiles() {
        assert!(super::process_for_import("x.png", b"not an image".to_vec()).is_none());
        assert!(super::process_for_import(".hidden.png", tiny_png()).is_none());
    }
}
