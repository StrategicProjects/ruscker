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
//! stays pure / unit-testable.

use anyhow::{anyhow, Context, Result};
use image::ImageReader;
use std::io::Cursor;

/// Maximum source upload size accepted. Anything bigger is
/// rejected before decoding — protects against
/// decompression-bomb-style memory attacks.
pub const MAX_UPLOAD_BYTES: usize = 10 * 1024 * 1024;

/// Soft target after re-encoding. Larger uploads still go through
/// but produce a `tracing::warn`. Used by callers to surface a UI
/// warning.
pub const SOFT_TARGET_BYTES: usize = 500 * 1024;

/// Output of [`process_upload`] — everything `db::images::insert`
/// needs and nothing it doesn't.
pub struct Processed {
    pub filename: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
    pub width: Option<u32>,
    pub height: Option<u32>,
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
            let (webp_bytes, w, h) = encode_to_webp(&bytes).context("convert to WebP")?;
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
            let dims = decode_dimensions(&bytes).ok();
            Ok(Processed {
                filename: rewrite_extension(&base, "webp"),
                mime_type: "image/webp".into(),
                bytes,
                width: dims.map(|(w, _)| w),
                height: dims.map(|(_, h)| h),
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
        SourceKind::Png => ("image/png", decode_dimensions(&bytes).ok()),
        SourceKind::Jpeg => ("image/jpeg", decode_dimensions(&bytes).ok()),
        SourceKind::Webp => ("image/webp", decode_dimensions(&bytes).ok()),
        SourceKind::Svg => ("image/svg+xml", None),
    };
    Some(Processed {
        filename,
        mime_type: mime.into(),
        bytes,
        width: dims.map(|(w, _)| w),
        height: dims.map(|(_, h)| h),
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

fn encode_to_webp(bytes: &[u8]) -> Result<(Vec<u8>, u32, u32)> {
    let img = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("guess format")?
        .decode()
        .context("decode source image")?;

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
    let img = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .context("guess format")?
        .decode()
        .context("decode source image")?;
    // `thumbnail` preserves aspect ratio and never upscales, using a
    // fast filter — fine for a small tile.
    let thumb = img.thumbnail(max_dim, max_dim);
    let mut out: Vec<u8> = Vec::new();
    thumb
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::WebP)
        .context("write WebP thumbnail")?;
    Ok(out)
}

fn decode_dimensions(bytes: &[u8]) -> Result<(u32, u32)> {
    let img = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()?
        .decode()?;
    Ok((img.width(), img.height()))
}

#[cfg(test)]
mod tests {
    use super::*;

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
