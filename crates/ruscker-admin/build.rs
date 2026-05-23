//! Build script for ruscker-admin.
//!
//! Responsibilities:
//! - Ensure the Tailwind 4 standalone CLI binary is available
//!   (download once per (version, platform) to `OUT_DIR`, or use
//!   the path in the `TAILWIND_BIN` env var if set).
//! - Compile `assets/tailwind/input.css` to `$OUT_DIR/styles.css`.
//!
//! The output CSS is included by `src/routes/assets.rs` via
//! `include_bytes!(concat!(env!("OUT_DIR"), "/styles.css"))`, so it
//! ships embedded in the binary — no separate asset deployment.
//!
//! ## Reproducibility
//!
//! Tailwind version is pinned in [`TAILWIND_VERSION`]. To bump it,
//! consult the latest stable release (PLAN.md §10 — never rely on
//! memory; check
//! <https://github.com/tailwindlabs/tailwindcss/releases>).
//!
//! ## Offline / air-gapped builds
//!
//! Set `TAILWIND_BIN=/path/to/tailwindcss` to skip the download and
//! use a pre-installed binary. CI caches the download under
//! `~/.cache/ruscker/tailwindcss-<ver>`.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const TAILWIND_VERSION: &str = "4.3.0";

fn main() {
    // Rerun triggers — only the things that actually affect the
    // generated CSS or the Tailwind binary choice.
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/tailwind/input.css");
    println!("cargo:rerun-if-changed=templates");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-env-changed=TAILWIND_BIN");
    println!("cargo:rerun-if-env-changed=TAILWIND_SKIP");

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR set by cargo"));
    let css_out = out_dir.join("styles.css");

    if env::var("TAILWIND_SKIP").is_ok() {
        // Still emit something so include_bytes! works in src/routes.
        if !css_out.exists() {
            fs::write(&css_out, b"/* tailwind skipped */\n").unwrap();
        }
        println!("cargo:warning=TAILWIND_SKIP set — emitted placeholder styles.css");
        return;
    }

    let bin = match env::var("TAILWIND_BIN") {
        Ok(p) => PathBuf::from(p),
        Err(_) => ensure_binary(),
    };

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let input = manifest_dir.join("assets/tailwind/input.css");

    let status = Command::new(&bin)
        .args(["-i".as_ref(), input.as_os_str(), "-o".as_ref(), css_out.as_os_str()])
        .arg("--minify")
        .status()
        .unwrap_or_else(|e| panic!("failed to invoke tailwind ({}): {e}", bin.display()));

    if !status.success() {
        panic!("tailwindcss compile failed (exit {:?})", status.code());
    }
}

/// Resolve the standalone Tailwind binary for the host platform,
/// downloading once and caching under `~/.cache/ruscker/`.
fn ensure_binary() -> PathBuf {
    let asset = host_asset_name();
    let cache_root = cache_dir().join("ruscker");
    let cache_path = cache_root.join(format!("tailwindcss-{TAILWIND_VERSION}-{asset}"));

    if cache_path.exists() {
        return cache_path;
    }

    fs::create_dir_all(&cache_root).expect("create cache dir");

    let url = format!(
        "https://github.com/tailwindlabs/tailwindcss/releases/download/v{TAILWIND_VERSION}/{asset}"
    );

    println!("cargo:warning=downloading tailwind {TAILWIND_VERSION} from {url}");

    // Use curl — universally available on macOS and standard Linux.
    let tmp = cache_path.with_extension("download");
    let status = Command::new("curl")
        .args(["-fL", "--retry", "3", "--retry-delay", "2", "--silent", "--show-error"])
        .arg("-o")
        .arg(&tmp)
        .arg(&url)
        .status()
        .expect("curl failed to start (is curl installed?)");
    if !status.success() {
        let _ = fs::remove_file(&tmp);
        panic!("curl download of {url} failed (exit {:?})", status.code());
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&tmp).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&tmp, perms).unwrap();
    }

    fs::rename(&tmp, &cache_path).expect("rename downloaded binary");
    cache_path
}

/// Asset filename for the current build host. Matches the naming
/// pattern from <https://github.com/tailwindlabs/tailwindcss/releases>.
fn host_asset_name() -> &'static str {
    let os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let env_abi = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    match (os.as_str(), arch.as_str(), env_abi.as_str()) {
        ("macos", "aarch64", _) => "tailwindcss-macos-arm64",
        ("macos", "x86_64", _) => "tailwindcss-macos-x64",
        ("linux", "aarch64", "musl") => "tailwindcss-linux-arm64-musl",
        ("linux", "aarch64", _) => "tailwindcss-linux-arm64",
        ("linux", "x86_64", "musl") => "tailwindcss-linux-x64-musl",
        ("linux", "x86_64", _) => "tailwindcss-linux-x64",
        // No Windows in scope (PLAN.md §6.1 lists it as out-of-scope).
        (o, a, e) => panic!("no Tailwind 4 standalone binary for {o}/{a}/{e}; set TAILWIND_BIN"),
    }
}

fn cache_dir() -> PathBuf {
    if let Ok(p) = env::var("XDG_CACHE_HOME") {
        return PathBuf::from(p);
    }
    let home = env::var("HOME").expect("HOME env var must be set");
    Path::new(&home).join(".cache")
}
