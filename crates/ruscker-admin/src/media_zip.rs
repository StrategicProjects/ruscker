//! Reading a `.zip` of images for the media library's batch import.
//!
//! Only the *unpacking* lives here, and it is synchronous and pure (bytes
//! in, files out) so it can be tested without a server: the route runs it
//! on a blocking thread and then sends every file through the same
//! pipeline as a single upload (`images::process_upload`), which is where
//! the content is actually validated.
//!
//! A zip is an archive somebody hands us, so nothing in it is trusted:
//!
//! - **Sizes are measured, not read from the header.** An entry's declared
//!   size is whatever its author wrote; each entry is read through a cap,
//!   and the whole archive through a total budget — the "zip bomb" guard.
//! - **Paths are ignored.** Only an entry's final name is kept (the media
//!   library is flat) and nothing is ever written to disk, so a path such
//!   as `../../etc/x.svg` has nowhere to climb to; such an entry is
//!   dropped anyway, with the hidden folders.
//! - **Only image extensions are unpacked.** Everything else is counted
//!   and left alone, without being decompressed.

use std::io::Read;

/// Largest `.zip` accepted. The route's body limit sits just above it.
pub const MAX_ZIP_BYTES: usize = 64 * 1024 * 1024;
/// Entries looked at in one archive. Above this the archive is refused
/// whole: silently importing "the first N" would be a surprise.
pub const MAX_ENTRIES: usize = 2000;
/// Budget for everything unpacked from one archive, in bytes.
pub const MAX_TOTAL_BYTES: u64 = 128 * 1024 * 1024;

/// One image file taken out of the archive.
#[derive(Debug)]
pub struct ZipFile {
    /// The entry's final path component (`logos/sepe.svg` → `sepe.svg`).
    pub name: String,
    pub bytes: Vec<u8>,
}

/// Why an entry that looked like an image was not unpacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// Larger than a single upload may be.
    TooLarge,
    /// Could not be read (encrypted, damaged, unsupported compression).
    Unreadable,
}

/// What came out of an archive.
#[derive(Debug, Default)]
pub struct Extracted {
    pub files: Vec<ZipFile>,
    /// Image-looking entries that were not unpacked, with the reason.
    pub skipped: Vec<(String, SkipReason)>,
    /// Entries that are not images at all (other extensions). Folders and
    /// operating-system litter (`__MACOSX/`, dot-files) are not counted.
    pub not_images: usize,
}

/// Why a whole archive was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZipRefusal {
    /// Not a readable zip archive.
    NotAZip,
    /// More than [`MAX_ENTRIES`] entries.
    TooManyEntries,
    /// The images in it unpack to more than [`MAX_TOTAL_BYTES`].
    TooLargeUnpacked,
}

/// The extensions the media library stores (see `images::SourceKind`).
fn is_image_name(name: &str) -> bool {
    let Some((_, ext)) = name.rsplit_once('.') else {
        return false;
    };
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "svg" | "png" | "jpg" | "jpeg" | "webp"
    )
}

/// The final component of an entry path, or `None` for what should not be
/// imported: a folder, a dot-file, or anything under a hidden or
/// `__MACOSX` folder (macOS stores a `._name` twin of every file there).
fn import_name(path: &str) -> Option<&str> {
    let path = path.trim_end_matches('/');
    let mut parts = path.split(['/', '\\']).filter(|p| !p.is_empty());
    let name = parts.next_back()?;
    if name.starts_with('.') {
        return None;
    }
    if parts.any(|dir| dir == "__MACOSX" || dir.starts_with('.')) {
        return None;
    }
    Some(name)
}

/// Unpack the image files of a zip archive. `per_file_cap` is the largest
/// a single file may be once unpacked (the single-upload limit).
pub fn extract_images(zip_bytes: &[u8], per_file_cap: usize) -> Result<Extracted, ZipRefusal> {
    extract_images_within(zip_bytes, per_file_cap, MAX_TOTAL_BYTES)
}

/// [`extract_images`] with the total budget as a parameter, so a test can
/// reach it with small files.
fn extract_images_within(
    zip_bytes: &[u8],
    per_file_cap: usize,
    total_cap: u64,
) -> Result<Extracted, ZipRefusal> {
    let mut archive =
        zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|_| ZipRefusal::NotAZip)?;
    if archive.len() > MAX_ENTRIES {
        return Err(ZipRefusal::TooManyEntries);
    }
    let mut out = Extracted::default();
    let mut total: u64 = 0;
    for index in 0..archive.len() {
        // The name first, from the directory alone — nothing is
        // decompressed for an entry that will not be imported.
        let (name, is_dir) = match archive.by_index_raw(index) {
            Ok(entry) => match entry.name() {
                Ok(path) => (import_name(&path).map(str::to_string), entry.is_dir()),
                // A name that cannot be decoded: nothing to call the file.
                Err(_) => continue,
            },
            Err(_) => continue,
        };
        let Some(name) = name else { continue };
        if is_dir {
            continue;
        }
        if !is_image_name(&name) {
            out.not_images += 1;
            continue;
        }
        let Ok(entry) = archive.by_index(index) else {
            out.skipped.push((name, SkipReason::Unreadable));
            continue;
        };
        // One byte past the cap tells "exactly at the cap" from "over it",
        // whatever size the entry claims to be — and never more than what
        // is left of the archive's budget (plus the same one byte).
        let left = total_cap.saturating_sub(total);
        let limit = (per_file_cap as u64).min(left) + 1;
        let mut bytes = Vec::new();
        let read = entry.take(limit).read_to_end(&mut bytes);
        // EVERY byte inflated counts, also for an entry that is then
        // skipped: otherwise an archive of many oversized entries would
        // make the server inflate far more than the budget, one skipped
        // entry at a time (security review of this change).
        total += bytes.len() as u64;
        if total > total_cap {
            return Err(ZipRefusal::TooLargeUnpacked);
        }
        match read {
            Err(_) => out.skipped.push((name, SkipReason::Unreadable)),
            Ok(_) if bytes.len() > per_file_cap => out.skipped.push((name, SkipReason::TooLarge)),
            Ok(_) => out.files.push(ZipFile { name, bytes }),
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Build a zip in memory from `(path, content)` pairs.
    fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (path, content) in entries {
            if path.ends_with('/') {
                w.add_directory(*path, opts).unwrap();
            } else {
                w.start_file(*path, opts).unwrap();
                w.write_all(content).unwrap();
            }
        }
        w.finish().unwrap().into_inner()
    }

    const SVG: &[u8] = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>";

    #[test]
    fn images_come_out_flat_and_the_rest_is_left_alone() {
        let zip = zip_of(&[
            ("logos/", b""),
            ("logos/sepe.svg", SVG),
            ("logos/sub/Brasao.SVG", SVG),
            ("capa.png", b"not really a png"),
            ("README.md", b"# hi"),
            ("__MACOSX/logos/._sepe.svg", b"resource fork"),
            (".hidden/x.svg", SVG),
            ("logos/.DS_Store", b"x"),
        ]);
        let got = extract_images(&zip, 1024).unwrap();
        let names: Vec<&str> = got.files.iter().map(|f| f.name.as_str()).collect();
        // The png is unpacked too: whether it IS one is the pipeline's call.
        assert_eq!(names, ["sepe.svg", "Brasao.SVG", "capa.png"]);
        assert_eq!(got.files[0].bytes, SVG);
        assert_eq!(got.not_images, 1, "README.md; system litter is not counted");
        assert!(got.skipped.is_empty());
    }

    /// Folders never reach the library: only the final name is kept. And
    /// an entry whose path climbs out of the archive (`..`) is dropped
    /// altogether — `..` counts as a hidden folder, and no honest archive
    /// has one.
    #[test]
    fn a_path_is_never_more_than_its_last_component() {
        let zip = zip_of(&[
            ("../../etc/logo.svg", SVG),
            ("a\\b\\win.svg", SVG),
            ("deep/er/unix.svg", SVG),
        ]);
        let got = extract_images(&zip, 1024).unwrap();
        let names: Vec<&str> = got.files.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["win.svg", "unix.svg"]);
    }

    /// The cap is applied to what is actually read: a highly compressible
    /// entry (a tiny zip of a huge file) is skipped, not unpacked whole.
    #[test]
    fn an_entry_over_the_cap_is_skipped_and_the_others_still_import() {
        let big = vec![b' '; 50_000];
        let zip = zip_of(&[("big.svg", &big), ("ok.svg", SVG)]);
        assert!(zip.len() < 2_000, "compresses to almost nothing: {}", zip.len());
        let got = extract_images(&zip, 10_000).unwrap();
        assert_eq!(got.skipped, vec![("big.svg".to_string(), SkipReason::TooLarge)]);
        assert_eq!(got.files.len(), 1);
        assert_eq!(got.files[0].name, "ok.svg");
        // Exactly at the cap is fine.
        let edge = vec![b' '; 10_000];
        let got = extract_images(&zip_of(&[("edge.svg", &edge)]), 10_000).unwrap();
        assert_eq!(got.files.len(), 1);
    }

    /// The archive's budget counts what was inflated, not what was kept:
    /// entries skipped for being too large still spend it, so many of
    /// them refuse the archive instead of being inflated one after another.
    #[test]
    fn skipped_entries_spend_the_budget_too() {
        let big = vec![b' '; 5_000];
        let entries: Vec<(String, &[u8])> =
            (0..10).map(|i| (format!("big-{i}.svg"), &big[..])).collect();
        let entries: Vec<(&str, &[u8])> = entries.iter().map(|(n, b)| (n.as_str(), *b)).collect();
        let zip = zip_of(&entries);
        // Per-file cap 1 000: each entry is read up to 1 001 bytes and
        // skipped. A budget of 4 000 is spent by the fourth one.
        assert_eq!(
            extract_images_within(&zip, 1_000, 4_000).unwrap_err(),
            ZipRefusal::TooLargeUnpacked
        );
        // With room for all ten reads, they are all just skipped.
        let got = extract_images_within(&zip, 1_000, 20_000).unwrap();
        assert_eq!(got.skipped.len(), 10);
        assert!(got.files.is_empty());
        // And kept files are bounded by the same budget.
        let small = vec![b' '; 900];
        let zip = zip_of(&[("a.svg", &small), ("b.svg", &small), ("c.svg", &small)]);
        assert_eq!(
            extract_images_within(&zip, 1_000, 2_000).unwrap_err(),
            ZipRefusal::TooLargeUnpacked
        );
    }

    #[test]
    fn what_is_not_a_zip_is_refused() {
        assert_eq!(extract_images(SVG, 1024).unwrap_err(), ZipRefusal::NotAZip);
        assert_eq!(extract_images(b"", 1024).unwrap_err(), ZipRefusal::NotAZip);
    }

    #[test]
    fn too_many_entries_refuse_the_whole_archive() {
        let names: Vec<String> = (0..=MAX_ENTRIES).map(|i| format!("{i}.txt")).collect();
        let entries: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b""[..])).collect();
        assert_eq!(
            extract_images(&zip_of(&entries), 1024).unwrap_err(),
            ZipRefusal::TooManyEntries
        );
    }
}
