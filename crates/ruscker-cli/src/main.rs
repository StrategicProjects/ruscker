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

    /// Import a YAML configuration into an admin database (SQLite via
    /// `--db`, or a shared Postgres catalog via `--config-db-url`).
    /// Idempotent — re-running with unchanged YAML produces zero
    /// writes. Specs in the DB but absent from the YAML are NOT
    /// deleted (separate operator action).
    Import {
        /// Path to the source YAML.
        path: PathBuf,

        /// Path to the SQLite file. Created with the schema if
        /// missing. Mutually exclusive with `--config-db-url`.
        #[arg(long)]
        db: Option<PathBuf>,

        /// Postgres DSN for the shared HA admin catalog
        /// (`postgres://…`). Seeds a fresh Postgres catalog from the
        /// YAML — the HA counterpart to `--db`. Takes precedence over
        /// `--db` when both are given.
        #[arg(long, env = "RUSCKER_CONFIG_DB_URL")]
        config_db_url: Option<String>,
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

        /// Force the Docker backend on. By default Ruscker auto-connects
        /// when the daemon socket is reachable; `--docker` makes a failed
        /// connect a fatal boot error instead of a landing-only fallback
        /// (useful for a remote/explicit Docker host).
        #[arg(long)]
        docker: bool,

        /// Run landing-only: never connect to Docker even if the socket
        /// is reachable. The proxy routes (`/app/*`, `/api/*`) return 503.
        /// Opts out of the default auto-connect.
        #[arg(long, conflicts_with = "docker")]
        no_docker: bool,

        /// Postgres DSN for the shared HA session store
        /// (`postgres://user:pass@host/db`). When set, several Ruscker
        /// instances behind a load balancer share one session table so
        /// their seat accounting and load balancing stay consistent.
        /// Omitted → the single-node in-memory store (default).
        #[arg(long, env = "RUSCKER_SESSION_STORE_URL")]
        session_store_url: Option<String>,

        /// Postgres DSN for the shared HA admin-session store (#185).
        /// When set, sign-in sessions survive a load-balancer hop
        /// between Ruscker instances — removes the need for the
        /// sticky-upstream workaround documented for #161. A short
        /// local TTL cache keeps the proxy hot path effectively
        /// DB-free. Omitted → the single-node in-memory store.
        #[arg(long, env = "RUSCKER_ADMIN_SESSION_STORE_URL")]
        admin_session_store_url: Option<String>,

        /// Serve the whole portal under a base path, for mounting behind
        /// a reverse proxy at a subpath when you can't make a subdomain
        /// (e.g. `--base-path /box` ⇒ `example.org/box/`). Overrides
        /// `server.context-path` in the config. Empty ⇒ root (default).
        #[arg(long, env = "RUSCKER_BASE_PATH")]
        base_path: Option<String>,
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
        Command::Import {
            path,
            db,
            config_db_url,
        } => cmd_import(&path, db, config_db_url),
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
            no_docker,
            session_store_url,
            admin_session_store_url,
            base_path,
        } => cmd_serve(ServeArgs {
            config_path: config,
            bind_override: bind,
            images_dir_override: images_dir,
            db_path: db,
            config_db_url,
            admin_token,
            master_key,
            docker,
            no_docker,
            session_store_url,
            admin_session_store_url,
            base_path_override: base_path,
            log_buffer,
        }),
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

fn cmd_import(
    yaml_path: &PathBuf,
    db_path: Option<PathBuf>,
    config_db_url: Option<String>,
) -> Result<()> {
    // Need a destination. Postgres wins if both are given.
    if config_db_url.is_none() && db_path.is_none() {
        anyhow::bail!(
            "no destination — pass --db <path> (SQLite) or \
             --config-db-url <postgres://…> (shared HA catalog)"
        );
    }

    let config = Config::from_file(yaml_path).with_context(|| {
        format!("failed to load config from {}", yaml_path.display())
    })?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    // Returns the import report plus a human label for the destination
    // (never the raw DSN — it may carry a password).
    let (report, target) = rt.block_on(async {
        if let Some(url) = config_db_url {
            if db_path.is_some() {
                eprintln!("note: both --config-db-url and --db given; using Postgres");
            }
            let pool = ruscker_admin::db::open_pg(&url)
                .await
                .context("connect to the Postgres admin catalog")?;
            let r = ruscker_admin::db::specs::import_all(
                &ruscker_admin::db::ConfigDb::Postgres(pool.clone()),
                &config,
            )
            .await?;
            pool.close().await;
            anyhow::Ok((r, "Postgres admin catalog".to_string()))
        } else {
            let path = db_path.expect("db_path present");
            let pool = ruscker_admin::db::open(&path).await?;
            let r = ruscker_admin::db::specs::import_all(
                &ruscker_admin::db::ConfigDb::Sqlite(pool.clone()),
                &config,
            )
            .await?;
            pool.close().await;
            anyhow::Ok((r, path.display().to_string()))
        }
    })?;

    println!();
    println!("  Ruscker import");
    println!("  ──────────────");
    println!("  from:  {}", yaml_path.display());
    println!("  into:  {}", target);
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

/// Default Docker socket path on Linux. Pulled out as a constant so
/// the boot-time check has a single source of truth (`is_local_docker_reachable`)
/// and the unit test can swap in a tempdir path.
const DEFAULT_DOCKER_SOCKET: &str = "/var/run/docker.sock";

/// Cheap, blocking probe: is the Docker socket reachable on this host?
///
/// Just checks the file system metadata for a Unix socket — does NOT
/// open a connection or talk to `dockerd`. That's enough to decide
/// whether to nudge the operator about `--docker`; the real connect
/// only happens when the flag is set.
fn is_local_docker_reachable() -> bool {
    docker_socket_at(DEFAULT_DOCKER_SOCKET)
}

#[cfg(unix)]
fn docker_socket_at(path: &str) -> bool {
    use std::os::unix::fs::FileTypeExt;
    std::fs::metadata(path)
        .map(|m| m.file_type().is_socket())
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn docker_socket_at(_path: &str) -> bool {
    // Windows / non-Unix: Docker Desktop exposes a named pipe; we don't
    // probe it here. The nudge is for the Debian / systemd path.
    false
}

/// All the knobs the `serve` subcommand reads. Packaged as a struct
/// rather than positional arguments because there are too many — the
/// clap `Serve` variant flattens into this on dispatch, and `cmd_serve`
/// reads it field-by-field.
struct ServeArgs {
    config_path: PathBuf,
    bind_override: Option<std::net::SocketAddr>,
    images_dir_override: Option<PathBuf>,
    db_path: Option<PathBuf>,
    config_db_url: Option<String>,
    admin_token: Option<String>,
    master_key: Option<String>,
    docker: bool,
    no_docker: bool,
    session_store_url: Option<String>,
    admin_session_store_url: Option<String>,
    base_path_override: Option<String>,
    log_buffer: ruscker_admin::logbuf::LogBuffer,
}

fn cmd_serve(args: ServeArgs) -> Result<()> {
    let ServeArgs {
        config_path,
        bind_override,
        images_dir_override,
        db_path,
        config_db_url,
        admin_token,
        master_key,
        docker,
        no_docker,
        session_store_url,
        admin_session_store_url,
        base_path_override,
        log_buffer,
    } = args;
    let config = Config::from_file(&config_path).with_context(|| {
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
        let found = discover_images_dir(&config_path, &config);
        if let Some(dir) = &found {
            tracing::info!(dir = %dir.display(), "auto-discovered images dir");
        }
        found
    });

    // Captured before `config` is moved into the server — picks the
    // multi-host backend when `proxy.hosts` is set (Phase 6).
    let hosts = config.proxy.hosts.clone();

    // Base path (#173): the `--base-path` flag wins over
    // `server.context-path`. Normalized once; `""` ⇒ root.
    let base_path = match base_path_override {
        Some(p) => ruscker_config::normalize_base_path(&p),
        None => config.server.effective_base_path(),
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let mut server = ruscker_admin::AdminServer::new(addr, config)?
            .with_log_buffer(log_buffer)
            .with_base_path(&base_path);
        if let Some(dir) = images_dir {
            server = server.with_images_dir(dir);
        }
        if let Some(token) = admin_token {
            server = server.with_admin_token(token);
        }
        if let Some(k) = master_key {
            server = server.with_master_key(k).context("invalid --master-key")?;
        }
        // Docker backend (#477): the default is **auto** — connect when
        // the daemon is reachable, so a fresh install just works. `--docker`
        // forces it on (a failed connect is then a fatal boot error, e.g.
        // for a remote host); `--no-docker` forces landing-only.
        if no_docker {
            // Explicit landing-only — nothing to wire, no nudge.
        } else if !hosts.is_empty() {
            // Multi-host is itself an explicit opt-in (the operator listed
            // hosts); `connect` validates them and fails only on zero.
            tracing::info!(count = hosts.len(), "multi-host backend");
            let backend = ruscker_docker::MultiHostDockerBackend::connect(&hosts)
                .context("connect to Docker hosts")?;
            server = server.with_backend(std::sync::Arc::new(backend));
        } else if docker || is_local_docker_reachable() {
            // `docker` ⇒ forced on; otherwise auto (the socket file exists).
            match ruscker_docker::LocalDockerBackend::local() {
                Ok(b) => {
                    let backend: std::sync::Arc<dyn ruscker_core::ContainerBackend> =
                        std::sync::Arc::new(b);
                    // The socket file can be present while the service user
                    // lacks access (not in the `docker` group) — that only
                    // surfaces on a real API call, so probe before committing.
                    match backend.list().await {
                        Ok(_) => server = server.with_backend(backend),
                        Err(e) if docker => {
                            return Err(anyhow::anyhow!("{e}"))
                                .context("--docker is set but the Docker daemon is unreachable");
                        }
                        Err(e) => tracing::warn!(
                            error = %e,
                            "Docker socket is present but the daemon is unreachable (the \
                             service user may not be in the `docker` group) — running \
                             landing-only; app containers won't spawn. Run \
                             `sudo ruscker-enable-docker` or add the user to the `docker` \
                             group; pass --no-docker to silence this."
                        ),
                    }
                }
                Err(e) if docker => return Err(e).context("connect to Docker daemon"),
                Err(e) => tracing::warn!(
                    error = ?e,
                    "could not initialise the Docker client — running landing-only"
                ),
            }
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
            // HA: elect a single scaler leader via a Postgres advisory
            // lock on the same database, so instances don't fight over
            // the replica count.
            server = server.with_leader_elector(std::sync::Arc::new(
                ruscker_admin::leader::PgLeaderLock::new(url),
            ));
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
        if let Some(url) = admin_session_store_url {
            tracing::info!("HA admin-session store: Postgres");
            let store = ruscker_admin::admin_sessions_pg::PostgresAdminSessionStore::connect(&url)
                .await
                .context("connect to the Postgres admin-session store")?;
            server = server.with_admin_session_store(std::sync::Arc::new(store));
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
    // verbosity flag only sets the default filter. The startup banner
    // (#452) rides a dedicated target raised to `info` so it shows even
    // at the default `warn` — `EnvFilter` prefers the longest target
    // prefix, so this un-mutes only the banner, not the whole app.
    let env = std::env::var("RUST_LOG").unwrap_or_else(|_| {
        format!(
            "ruscker={level},{}=info",
            ruscker_admin::STARTUP_LOG_TARGET
        )
    });
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

fn print_human_report(path: &Path, config: &Config, report: &ValidationReport) {
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
        // Real access rule (`Spec::is_open()`), not the retired
        // decorative `template-properties.icon` flag (#346).
        let access = if spec.is_open() { "open" } else { "restrict" };
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

/// Truncate `s` to at most `n` characters, appending an ellipsis when
/// it was cut. Counts and slices on `char` boundaries — a byte-index
/// slice (`&s[..k]`) would panic if the cut landed mid-UTF-8-codepoint
/// (e.g. a spec id with accented characters) (#327).
fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let head: String = s.chars().take(n.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_counts_chars_and_never_panics_on_multibyte() {
        // Short ASCII passes through.
        assert_eq!(truncate("shiny", 25), "shiny");
        // ASCII over the cap gets an ellipsis at n-1 chars.
        assert_eq!(truncate("abcdef", 4), "abc…");
        // #327: a multibyte string cut mid-codepoint must not panic. The
        // old byte-slice `&s[..k]` panicked when k landed inside a
        // multi-byte char (each `ç`/`ã` is 2 bytes here).
        let acc = "açãoçãoçãoção"; // 12 chars, > 6
        let out = truncate(acc, 6);
        assert_eq!(out.chars().count(), 6); // 5 kept + ellipsis
        assert!(out.ends_with('…'));
        // Boundary: exactly at the cap is left intact.
        assert_eq!(truncate("ção", 3), "ção");
    }

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

    #[cfg(unix)]
    #[test]
    fn docker_socket_at_detects_a_unix_socket() {
        use std::os::unix::net::UnixListener;
        let tmp = std::env::temp_dir().join(format!(
            "ruscker-sock-{}-{}.sock",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_file(&tmp);

        // Missing → false.
        assert!(!docker_socket_at(tmp.to_str().unwrap()));

        // Bind a real Unix socket → true.
        let _listener = UnixListener::bind(&tmp).expect("bind unix socket");
        assert!(docker_socket_at(tmp.to_str().unwrap()));

        // A regular file under the same name → false.
        std::fs::remove_file(&tmp).unwrap();
        std::fs::write(&tmp, b"not a socket").unwrap();
        assert!(!docker_socket_at(tmp.to_str().unwrap()));

        std::fs::remove_file(&tmp).ok();
    }
}
