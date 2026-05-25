//! Ruscker CLI entry point.
//!
//! Currently supports:
//!
//! - `ruscker validate <path>` — parse and validate a YAML config, with
//!   a human-friendly report.
//! - `ruscker validate <path> --json` — same report as JSON for tooling.
//! - `ruscker show <path>` — render the interpolated config (for
//!   debugging env var interpolation).
//!
//! Future commands (see `docs/ROADMAP.md`):
//!
//! - `ruscker serve` — run the proxy
//! - `ruscker admin` — run the admin panel
//! - `ruscker import <path>` — import YAML into a SQLite database
//! - `ruscker export <path>` — export DB back to YAML

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use ruscker_config::{CompatWarning, Config, SpecKind, ValidationReport, Warning};
use std::path::PathBuf;

/// How log lines are rendered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, ValueEnum)]
enum LogFormat {
    /// Human-readable single-line output (default) — for terminals
    /// and `journalctl`.
    #[default]
    Text,
    /// One JSON object per line — for log shippers (Loki, Fluent
    /// Bit, the ELK stack) that parse structured fields.
    Json,
}

#[derive(Parser, Debug)]
#[command(
    name = "ruscker",
    version,
    about = "A lightweight Rust alternative to ShinyProxy and Shiny Server Free",
    long_about = None,
)]
struct Cli {
    /// Increase output verbosity (`-v`, `-vv`, `-vvv`)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    /// Log output format. `text` for humans, `json` for log
    /// aggregators. Also settable via `RUSCKER_LOG_FORMAT`.
    #[arg(long, value_enum, default_value_t = LogFormat::Text, env = "RUSCKER_LOG_FORMAT", global = true)]
    log_format: LogFormat,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Parse and validate a YAML configuration file
    Validate {
        /// Path to the application.yml (or any compatible config)
        path: PathBuf,

        /// Emit the validation report as JSON instead of human format
        #[arg(long)]
        json: bool,

        /// Exit non-zero if any warnings are produced
        #[arg(long)]
        strict: bool,

        /// Also report ShinyProxy features this config uses that
        /// Ruscker does not support (auth schemes, `volumes`,
        /// `kubernetes-*`, …). Exits non-zero if any are found.
        #[arg(long)]
        strict_compat: bool,
    },

    /// Render the YAML with environment variables interpolated, for
    /// debugging. Credentials are NOT redacted — only run in a safe shell.
    Show {
        path: PathBuf,
    },

    /// Print the resolved config as JSON (after interpolation + parsing)
    Inspect {
        path: PathBuf,
    },

    /// Import a YAML configuration into a SQLite admin database.
    /// Idempotent — re-running with unchanged YAML produces zero
    /// writes. Specs in the DB but absent from the YAML are NOT
    /// deleted (separate operator action).
    Import {
        /// Path to the source YAML.
        path: PathBuf,

        /// Path to the SQLite file. Created with the schema if
        /// missing.
        #[arg(long)]
        db: PathBuf,
    },

    /// Reconstruct an application.yml from a SQLite admin database
    /// and write it to stdout. Pipes naturally into `> backup.yml`
    /// or `| diff -u current.yml -` for change auditing.
    Export {
        /// Path to the SQLite file.
        #[arg(long)]
        db: PathBuf,
    },

    /// Start the HTTP server (public landing in Phase 1; admin + proxy
    /// land in Phase 2+).
    Serve {
        /// Path to application.yml
        #[arg(long, default_value = "application.yml")]
        config: PathBuf,

        /// Bind address override. Defaults to the value in the YAML.
        #[arg(long)]
        bind: Option<std::net::SocketAddr>,

        /// Directory served at /assets/img/. Defaults to
        /// `<config-dir>/assets/img/` if that path exists, otherwise
        /// no image route is mounted (cards fall back to tint-only).
        #[arg(long)]
        images_dir: Option<PathBuf>,

        /// Path to the SQLite admin database. Required for `/admin/*`
        /// routes to function. Without it, those routes return 503.
        #[arg(long)]
        db: Option<PathBuf>,

        /// Admin auth token. Overrides `RUSCKER_ADMIN_TOKEN` env var
        /// when set. Without either, /admin/* routes return 503.
        #[arg(long, env = "RUSCKER_ADMIN_TOKEN")]
        admin_token: Option<String>,

        /// 32-byte master key for the credentials store. Accepts
        /// hex (64 chars) or base64 (44 chars). Generate with
        /// `openssl rand -hex 32`. Without this set,
        /// /admin/credentials shows a hint instead of the form.
        #[arg(long, env = "RUSCKER_MASTER_KEY")]
        master_key: Option<String>,

        /// Enable the local Docker backend. Required for the
        /// proxy routes (`/app/*`, `/api/*`) to actually spawn
        /// containers. Without it those routes return 503.
        #[arg(long)]
        docker: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose, cli.log_format);

    match cli.command {
        Command::Validate {
            path,
            json,
            strict,
            strict_compat,
        } => cmd_validate(&path, json, strict, strict_compat),
        Command::Show { path } => cmd_show(&path),
        Command::Inspect { path } => cmd_inspect(&path),
        Command::Import { path, db } => cmd_import(&path, &db),
        Command::Export { db } => cmd_export(&db),
        Command::Serve { config, bind, images_dir, db, admin_token, master_key, docker } => {
            cmd_serve(&config, bind, images_dir, db, admin_token, master_key, docker)
        }
    }
}

fn cmd_export(db_path: &PathBuf) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let config = rt.block_on(async {
        let pool = ruscker_admin::db::open(db_path).await?;
        let c = ruscker_admin::db::export::reconstruct_config(&pool).await?;
        pool.close().await;
        anyhow::Ok(c)
    })?;

    // serde_yaml_ng's `to_string` emits a leading "---" document
    // separator and trailing newline — both expected for YAML.
    let yaml = serde_yaml_ng::to_string(&config).context("serialize Config to YAML")?;
    print!("{yaml}");
    Ok(())
}

fn cmd_import(yaml_path: &PathBuf, db_path: &PathBuf) -> Result<()> {
    let config = Config::from_file(yaml_path).with_context(|| {
        format!("failed to load config from {}", yaml_path.display())
    })?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    let report = rt.block_on(async {
        let pool = ruscker_admin::db::open(db_path).await?;
        let r = ruscker_admin::db::specs::import_all(&pool, &config).await?;
        pool.close().await;
        anyhow::Ok(r)
    })?;

    println!();
    println!("  Ruscker import");
    println!("  ──────────────");
    println!("  from:  {}", yaml_path.display());
    println!("  into:  {}", db_path.display());
    println!();
    println!("  specs:");
    println!("    created    {:>4}", report.created);
    println!("    updated    {:>4}", report.updated);
    println!("    unchanged  {:>4}", report.unchanged);
    println!();
    println!("  ✓ done. {} specs in the DB.",
        report.created + report.updated + report.unchanged);
    Ok(())
}

fn cmd_serve(
    config_path: &PathBuf,
    bind_override: Option<std::net::SocketAddr>,
    images_dir_override: Option<PathBuf>,
    db_path: Option<PathBuf>,
    admin_token: Option<String>,
    master_key: Option<String>,
    docker: bool,
) -> Result<()> {
    let config = Config::from_file(config_path).with_context(|| {
        format!("failed to load config from {}", config_path.display())
    })?;

    let addr = match bind_override {
        Some(a) => a,
        None => {
            let ip: std::net::IpAddr = config.proxy.bind_address.parse().with_context(|| {
                format!("invalid bind-address `{}`", config.proxy.bind_address)
            })?;
            std::net::SocketAddr::new(ip, config.proxy.port)
        }
    };

    // Default-discover images: look next to the config under
    // `assets/img/`. Matches the ShinyProxy templates/mlk/assets/img
    // layout, just relative to the YAML location.
    let images_dir = images_dir_override.or_else(|| {
        let candidate = config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("assets/img");
        candidate.is_dir().then_some(candidate)
    });

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let mut server = ruscker_admin::AdminServer::new(addr, config)?;
        if let Some(dir) = images_dir {
            server = server.with_images_dir(dir);
        }
        if let Some(token) = admin_token {
            server = server.with_admin_token(token);
        }
        if let Some(k) = master_key {
            server = server.with_master_key(k).context("invalid --master-key")?;
        }
        if docker {
            let backend = ruscker_docker::LocalDockerBackend::local()
                .context("connect to Docker daemon")?;
            server = server.with_backend(std::sync::Arc::new(backend));
        }
        if let Some(path) = db_path {
            let pool = ruscker_admin::db::open(&path).await?;
            server = server.with_db(pool);
        }
        server.run().await
    })
}

fn init_tracing(verbosity: u8, format: LogFormat) {
    let level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    // `RUST_LOG` always wins when set (operators expect it); the
    // verbosity flag only sets the default filter.
    let env = std::env::var("RUST_LOG").unwrap_or_else(|_| format!("ruscker={level}"));
    let builder = tracing_subscriber::fmt().with_env_filter(env);
    match format {
        // `.json()` and the default formatter return different
        // builder types, so each arm must call `.init()` itself.
        LogFormat::Text => builder.with_target(false).init(),
        LogFormat::Json => builder
            // Keep the module target in structured logs — it's a
            // cheap, high-value field for filtering in a shipper.
            .json()
            // Lift the event's fields to the top level instead of
            // nesting them under `"fields"`, so queries like
            // `addr="…"` work without a path prefix.
            .flatten_event(true)
            .init(),
    }
}

fn cmd_validate(path: &PathBuf, json: bool, strict: bool, strict_compat: bool) -> Result<()> {
    let config = Config::from_file(path)
        .with_context(|| format!("failed to load config from {}", path.display()))?;
    let report = config.validate();

    // Compatibility scan needs the raw YAML — the unsupported fields
    // it looks for are dropped by serde at parse time. Only run it
    // when asked; an empty vec otherwise keeps the output unchanged.
    let compat = if strict_compat {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        ruscker_config::validate::compat_scan(&config, &raw)
    } else {
        Vec::new()
    };

    if json {
        let payload = serde_json::json!({
            "path": path,
            "ok": report.is_clean(),
            "report": report,
            "compat_ok": compat.is_empty(),
            "compat": compat,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        print_human_report(path, &config, &report);
        if strict_compat {
            print_compat_report(&compat);
        }
    }

    if (strict && !report.is_clean()) || (strict_compat && !compat.is_empty()) {
        std::process::exit(2);
    }
    Ok(())
}

fn print_compat_report(compat: &[CompatWarning]) {
    println!("  ShinyProxy compatibility");
    println!("  ────────────────────────");
    if compat.is_empty() {
        println!("  ✓ no unsupported features");
    } else {
        println!("  ⚠ {} unsupported feature(s):", compat.len());
        for w in compat {
            println!("    - {}", format_compat_warning(w));
        }
    }
    println!();
}

fn format_compat_warning(w: &CompatWarning) -> String {
    match w {
        CompatWarning::UnsupportedAuth { scheme } => {
            format!("authentication '{scheme}' is not supported — only `none` (auth inside apps)")
        }
        CompatWarning::UnsupportedSpecField {
            spec_id,
            field,
            note,
        } => format!("spec {spec_id}: `{field}` — {note}"),
        CompatWarning::UnsupportedProxyField { field, note } => {
            format!("proxy.{field} — {note}")
        }
    }
}

fn cmd_show(path: &PathBuf) -> Result<()> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let rendered = ruscker_config::env::interpolate(&raw)?;
    print!("{rendered}");
    Ok(())
}

fn cmd_inspect(path: &PathBuf) -> Result<()> {
    let config = Config::from_file(path)?;
    println!("{}", serde_json::to_string_pretty(&config)?);
    Ok(())
}

fn print_human_report(path: &PathBuf, config: &Config, report: &ValidationReport) {
    println!();
    println!("  Ruscker config validation");
    println!("  ─────────────────────────");
    println!("  file: {}", path.display());
    println!("  title: {}", config.proxy.title.trim());
    println!(
        "  bind: {}:{}",
        config.proxy.bind_address, config.proxy.port
    );
    println!("  authentication: {:?}", config.proxy.authentication);
    println!();
    println!("  Specs: {} total", report.stats.total_specs);

    let mut kinds: Vec<_> = report.stats.by_kind.iter().collect();
    kinds.sort_by(|a, b| a.0.cmp(b.0));
    for (kind, count) in kinds {
        println!("    {:<14} {}", kind, count);
    }

    if !report.stats.by_state.is_empty() {
        println!();
        println!("  State:");
        let mut states: Vec<_> = report.stats.by_state.iter().collect();
        states.sort_by(|a, b| a.0.cmp(b.0));
        for (state, count) in states {
            println!("    {:<14} {}", state, count);
        }
    }

    println!();
    if report.warnings.is_empty() {
        println!("  ✓ no warnings");
    } else {
        println!("  ⚠ {} warning(s):", report.warnings.len());
        for w in &report.warnings {
            println!("    - {}", format_warning(w));
        }
    }
    println!();

    print_breakdown(config);
}

fn format_warning(w: &Warning) -> String {
    match w {
        Warning::DuplicateSpecId { id } => format!("duplicate spec id: {id}"),
        Warning::EmptyDisplayName { spec_id } => {
            format!("spec {spec_id} has no display-name")
        }
        Warning::EmptyDescription { spec_id } => {
            format!("spec {spec_id} has no description")
        }
        Warning::EmbeddedCredential { field, line } => {
            format!("embedded credential in '{field}' at line {line} — use ${{ENV_VAR}}")
        }
        Warning::UnknownTypeProperty { spec_id, value } => {
            format!(
                "spec {spec_id} uses unknown template-properties.type '{value}'"
            )
        }
        Warning::InvalidReplicaRange { spec_id, min, max } => {
            format!("spec {spec_id}: max-replicas ({max}) < min-replicas ({min})")
        }
        Warning::InvalidScaleThreshold {
            spec_id,
            scale_up,
            scale_down,
        } => {
            format!(
                "spec {spec_id}: scale thresholds invalid (up={scale_up:.2}, down={scale_down:.2})"
            )
        }
        Warning::SpecLackingContainerHasContainerFields { spec_id } => {
            format!("spec {spec_id} has no container-image but uses container-only fields")
        }
        Warning::InvalidRateLimit { spec_id, value } => {
            format!(
                "spec {spec_id} has an invalid api.rate-limit `{value}` \
                 (expected `N/unit`, e.g. `100/min`) — no limit will be applied"
            )
        }
    }
}

fn print_breakdown(config: &Config) {
    println!("  Spec breakdown:");
    println!(
        "    {:<26} {:<8} {:<8} {:<8} {:<8}",
        "id", "kind", "state", "access", "seats"
    );
    println!("    {}", "─".repeat(64));

    for spec in &config.proxy.specs {
        let kind = match spec.kind() {
            SpecKind::Shiny => "shiny",
            SpecKind::InteractiveApp => "interact",
            SpecKind::Api => "api",
            SpecKind::External => "external",
        };
        let state = spec.template_properties.state();
        let access = spec.template_properties.get_str("icon").unwrap_or("-");
        let seats = spec.effective_seats();
        println!(
            "    {:<26} {:<8} {:<8} {:<8} {:<8}",
            truncate(&spec.id, 25),
            kind,
            state,
            access,
            seats
        );
    }
    println!();
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n.saturating_sub(1)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_format_defaults_to_text() {
        let cli = Cli::try_parse_from(["ruscker", "validate", "app.yml"]).unwrap();
        assert_eq!(cli.log_format, LogFormat::Text);
    }

    #[test]
    fn log_format_accepts_json() {
        let cli =
            Cli::try_parse_from(["ruscker", "--log-format", "json", "validate", "app.yml"]).unwrap();
        assert_eq!(cli.log_format, LogFormat::Json);
    }

    #[test]
    fn log_format_rejects_unknown_value() {
        assert!(
            Cli::try_parse_from(["ruscker", "--log-format", "xml", "validate", "app.yml"]).is_err()
        );
    }
}
