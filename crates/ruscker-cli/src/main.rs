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
use std::path::{Path, PathBuf};

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

        /// Directory served at /assets/img/. When omitted, auto-
        /// discovered next to the config: `<config-dir>/assets/img/`,
        /// then `<config-dir>/<template-path>/assets/img/` (the
        /// ShinyProxy layout). If none exist, no image route is
        /// mounted (cards fall back to tint-only covers).
        #[arg(long)]
        images_dir: Option<PathBuf>,

        /// Path to the SQLite admin database. Required for `/admin/*`
        /// routes to function. Without it (and without
        /// `--config-db-url`), those routes return 503.
        #[arg(long)]
        db: Option<PathBuf>,

        /// Postgres DSN for the shared admin catalog (`postgres://…`).
        /// HA alternative to `--db`: several instances share one
        /// editable spec catalog / users / landing / audit store.
        /// Takes precedence over `--db` when both are set.
        #[arg(long, env = "RUSCKER_CONFIG_DB_URL")]
        config_db_url: Option<String>,

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

        /// Postgres DSN for the shared HA session store
        /// (`postgres://user:pass@host/db`). When set, several Ruscker
        /// instances behind a load balancer share one session table so
        /// their seat accounting and load balancing stay consistent.
        /// Omitted → the single-node in-memory store (default).
        #[arg(long, env = "RUSCKER_SESSION_STORE_URL")]
        session_store_url: Option<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let log_buffer = init_tracing(cli.verbose, cli.log_format);

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
        Command::Serve {
            config,
            bind,
            images_dir,
            db,
            config_db_url,
            admin_token,
            master_key,
            docker,
            session_store_url,
        } => cmd_serve(
            &config,
            bind,
            images_dir,
            db,
            config_db_url,
            admin_token,
            master_key,
            docker,
            session_store_url,
            log_buffer,
        ),
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
        let r = ruscker_admin::db::specs::import_all(
            &ruscker_admin::db::ConfigDb::Sqlite(pool.clone()),
            &config,
        )
        .await?;
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

/// Auto-discover the `/assets/img/` source directory beside a config
/// when `--images-dir` isn't given. Tries, in order:
///
/// 1. `<config-dir>/assets/img/` — assets sitting next to the YAML.
/// 2. `<config-dir>/<proxy.template-path>/assets/img/` — the ShinyProxy
///    `template-path` layout (e.g. `templates/mlk`), so a config left
///    in place after migration finds its card logos with no flag.
///
/// Returns the first existing directory, or `None`.
fn discover_images_dir(config_path: &Path, config: &Config) -> Option<PathBuf> {
    let base = config_path.parent().unwrap_or_else(|| Path::new("."));

    let direct = base.join("assets/img");
    if direct.is_dir() {
        return Some(direct);
    }

    if let Some(tp) = config.proxy.template_path.as_ref() {
        let tp_dir = if tp.is_absolute() {
            tp.clone()
        } else {
            base.join(tp)
        };
        let candidate = tp_dir.join("assets/img");
        if candidate.is_dir() {
            return Some(candidate);
        }
    }

    None
}

fn cmd_serve(
    config_path: &PathBuf,
    bind_override: Option<std::net::SocketAddr>,
    images_dir_override: Option<PathBuf>,
    db_path: Option<PathBuf>,
    config_db_url: Option<String>,
    admin_token: Option<String>,
    master_key: Option<String>,
    docker: bool,
    session_store_url: Option<String>,
    log_buffer: ruscker_admin::logbuf::LogBuffer,
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

    // Resolve the images dir: explicit flag wins, else auto-discover
    // next to the config / under the ShinyProxy template-path.
    let images_dir = images_dir_override.or_else(|| {
        let found = discover_images_dir(config_path, &config);
        if let Some(dir) = &found {
            tracing::info!(dir = %dir.display(), "auto-discovered images dir");
        }
        found
    });

    // Captured before `config` is moved into the server — picks the
    // multi-host backend when `proxy.hosts` is set (Phase 6).
    let hosts = config.proxy.hosts.clone();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let mut server = ruscker_admin::AdminServer::new(addr, config)?.with_log_buffer(log_buffer);
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
            let backend: std::sync::Arc<dyn ruscker_core::ContainerBackend> = if hosts.is_empty() {
                std::sync::Arc::new(
                    ruscker_docker::LocalDockerBackend::local()
                        .context("connect to Docker daemon")?,
                )
            } else {
                tracing::info!(count = hosts.len(), "multi-host backend");
                std::sync::Arc::new(
                    ruscker_docker::MultiHostDockerBackend::connect(&hosts)
                        .context("connect to Docker hosts")?,
                )
            };
            server = server.with_backend(backend);
        }
        // Config database: Postgres (shared HA catalog) takes precedence
        // over the SQLite path when both are given.
        if let Some(url) = config_db_url {
            if db_path.is_some() {
                tracing::warn!("both --config-db-url and --db set; using Postgres");
            }
            tracing::info!("admin catalog: Postgres");
            let pool = ruscker_admin::db::open_pg(&url)
                .await
                .context("connect to the Postgres admin catalog")?;
            server = server.with_config_db(ruscker_admin::db::ConfigDb::Postgres(pool));
        } else if let Some(path) = db_path {
            let pool = ruscker_admin::db::open(&path).await?;
            server = server.with_db(pool);
        }
        if let Some(url) = session_store_url {
            tracing::info!("HA session store: Postgres");
            let store = ruscker_admin::sessions_pg::PostgresSessionStore::connect(&url)
                .await
                .context("connect to the Postgres session store")?;
            server = server.with_session_store(std::sync::Arc::new(store));
        }
        server.run().await
    })
}

/// Initialise tracing and return the in-memory log buffer feeding the
/// admin "Logs" tab. Logs go to stdout (text or JSON) **and** to a
/// bounded ring buffer (always plain text), both behind the same
/// `EnvFilter`. Only `serve` consumes the returned buffer; other
/// commands just let it collect harmlessly.
fn init_tracing(verbosity: u8, format: LogFormat) -> ruscker_admin::logbuf::LogBuffer {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    // `RUST_LOG` always wins when set (operators expect it); the
    // verbosity flag only sets the default filter.
    let env = std::env::var("RUST_LOG").unwrap_or_else(|_| format!("ruscker={level}"));
    let filter = tracing_subscriber::EnvFilter::new(env);

    let buffer = ruscker_admin::logbuf::LogBuffer::new(2000);
    let registry = tracing_subscriber::registry().with(filter);
    // The ring-buffer layer is inlined per arm: the two stdout formats
    // give the layered subscriber different types, so its `S` parameter
    // must be inferred independently at each call site.
    match format {
        LogFormat::Text => registry
            .with(tracing_subscriber::fmt::layer().with_target(false))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_target(false)
                    .with_writer(BufMakeWriter(buffer.clone())),
            )
            .init(),
        LogFormat::Json => registry
            // Keep the module target + lift fields to the top level so
            // `addr="…"` queries work without a path prefix in a shipper.
            .with(tracing_subscriber::fmt::layer().json().flatten_event(true))
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_target(false)
                    .with_writer(BufMakeWriter(buffer.clone())),
            )
            .init(),
    }
    buffer
}

/// `MakeWriter` that funnels formatted log lines into the [`LogBuffer`].
#[derive(Clone)]
struct BufMakeWriter(ruscker_admin::logbuf::LogBuffer);

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufMakeWriter {
    type Writer = BufLineWriter;
    fn make_writer(&'a self) -> Self::Writer {
        BufLineWriter {
            buf: self.0.clone(),
            pending: Vec::new(),
        }
    }
}

/// Per-event writer: accumulates bytes and pushes each complete line
/// (the fmt layer writes one newline-terminated line per event).
struct BufLineWriter {
    buf: ruscker_admin::logbuf::LogBuffer,
    pending: Vec<u8>,
}

impl BufLineWriter {
    fn drain_lines(&mut self) {
        while let Some(pos) = self.pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=pos).collect();
            let s = String::from_utf8_lossy(&line);
            let s = s.trim_end();
            if !s.is_empty() {
                self.buf.push_line(s.to_string());
            }
        }
    }
}

impl std::io::Write for BufLineWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(data);
        self.drain_lines();
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for BufLineWriter {
    fn drop(&mut self) {
        // Flush a trailing line with no newline (defensive).
        let s = String::from_utf8_lossy(&self.pending);
        let s = s.trim_end();
        if !s.is_empty() {
            self.buf.push_line(s.to_string());
        }
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
        Warning::InvalidMaxBodySize { location, value } => {
            format!(
                "{location} has an invalid max-body-size `{value}` \
                 (expected a size like `10m`, `1g`, or plain bytes) — no limit will be applied"
            )
        }
        Warning::InvalidCpuLimit {
            spec_id,
            field,
            value,
        } => {
            format!(
                "spec {spec_id} has an invalid container-cpu-{field} `{value}` \
                 (expected a positive number of CPUs, e.g. `0.5`) — no CPU limit will be applied"
            )
        }
        Warning::InvalidMemoryLimit {
            spec_id,
            field,
            value,
        } => {
            format!(
                "spec {spec_id} has an invalid container-memory-{field} `{value}` \
                 (expected a size like `512m`, `1.5g`, or plain bytes) — no memory limit will be applied"
            )
        }
        Warning::ZeroSeats { spec_id } => {
            format!(
                "spec {spec_id} sets seats-per-container: 0 — a replica then looks \
                 saturated and idle at once, confusing the auto-scaler; use 1 or more"
            )
        }
        Warning::InvalidVolume { spec_id, value } => {
            format!(
                "spec {spec_id} has an invalid volume `{value}` \
                 (expected `/host:/container` or `/host:/container:ro`) — the mount will be skipped"
            )
        }
        Warning::InvalidHost { host, reason } => {
            format!("docker host `{host}`: {reason} — it will fail to connect at startup")
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

    #[test]
    fn discovers_template_path_assets_img() {
        use std::fs;
        let tmp = std::env::temp_dir().join(format!("ruscker-disc-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let cfg_dir = tmp.join("etc");
        let tp_img = cfg_dir.join("templates/mlk/assets/img");
        fs::create_dir_all(&tp_img).unwrap();
        let cfg_path = cfg_dir.join("application.yml");
        let config =
            Config::from_yaml("proxy:\n  template-path: templates/mlk\n  specs: []\n").unwrap();

        // Falls through to <config-dir>/<template-path>/assets/img.
        assert_eq!(discover_images_dir(&cfg_path, &config), Some(tp_img));

        // A direct <config-dir>/assets/img wins over the template-path.
        let direct = cfg_dir.join("assets/img");
        fs::create_dir_all(&direct).unwrap();
        assert_eq!(discover_images_dir(&cfg_path, &config), Some(direct));

        fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn discovers_none_when_no_assets() {
        let tmp = std::env::temp_dir().join(format!("ruscker-disc-none-{}", std::process::id()));
        let cfg_path = tmp.join("application.yml");
        let config =
            Config::from_yaml("proxy:\n  template-path: templates/x\n  specs: []\n").unwrap();
        assert!(discover_images_dir(&cfg_path, &config).is_none());
    }
}
