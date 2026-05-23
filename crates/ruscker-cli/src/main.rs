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
use clap::{Parser, Subcommand};
use ruscker_config::{Config, SpecKind, ValidationReport, Warning};
use std::path::PathBuf;

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

    /// Start the HTTP server (public landing in Phase 1; admin + proxy
    /// land in Phase 2+).
    Serve {
        /// Path to application.yml
        #[arg(long, default_value = "application.yml")]
        config: PathBuf,

        /// Bind address override. Defaults to the value in the YAML.
        #[arg(long)]
        bind: Option<std::net::SocketAddr>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match cli.command {
        Command::Validate { path, json, strict } => cmd_validate(&path, json, strict),
        Command::Show { path } => cmd_show(&path),
        Command::Inspect { path } => cmd_inspect(&path),
        Command::Serve { config, bind } => cmd_serve(&config, bind),
    }
}

fn cmd_serve(config_path: &PathBuf, bind_override: Option<std::net::SocketAddr>) -> Result<()> {
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

    let server = ruscker_admin::AdminServer::new(addr, config)?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(server.run())
}

fn init_tracing(verbosity: u8) {
    let level = match verbosity {
        0 => "warn",
        1 => "info",
        2 => "debug",
        _ => "trace",
    };
    let env = std::env::var("RUST_LOG").unwrap_or_else(|_| format!("ruscker={level}"));
    tracing_subscriber::fmt()
        .with_env_filter(env)
        .with_target(false)
        .init();
}

fn cmd_validate(path: &PathBuf, json: bool, strict: bool) -> Result<()> {
    let config = Config::from_file(path)
        .with_context(|| format!("failed to load config from {}", path.display()))?;
    let report = config.validate();

    if json {
        let payload = serde_json::json!({
            "path": path,
            "ok": report.is_clean(),
            "report": report,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
    } else {
        print_human_report(path, &config, &report);
    }

    if strict && !report.is_clean() {
        std::process::exit(2);
    }
    Ok(())
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
