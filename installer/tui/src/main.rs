use std::fs::File;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Parser;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

mod config;
mod disk;
mod encryption;
mod firstboot;
mod hostname_template;
mod net;
mod plan;
mod run;
mod script;
mod timezone;
mod ui;
mod util;
mod writer;

const DEFAULT_LOG_PATH: &str = "/var/log/bes-installer.log";

#[derive(Parser)]
#[command(name = "bes-installer", about = "BES Linux Images Installer")]
pub(crate) struct Cli {
    /// Path to config file (overrides automatic EFI partition search)
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// Path to log file (default: /var/log/bes-installer.log)
    #[arg(long, default_value = DEFAULT_LOG_PATH)]
    pub log: PathBuf,

    // r[impl installer.dryrun]
    /// Dry-run mode: collect all decisions and emit an install plan as JSON
    /// instead of performing any destructive operations.
    #[arg(long)]
    pub dry_run: bool,

    // r[impl installer.dryrun.output]
    /// Path to write the dry-run JSON install plan. If omitted, the plan is
    /// written to stdout.
    #[arg(long)]
    pub dry_run_output: Option<PathBuf>,

    // r[impl installer.dryrun.devices]
    /// Path to a JSON file describing fake block devices (for testing).
    /// When given, the installer reads devices from this file instead of
    /// running lsblk.
    #[arg(long)]
    pub fake_devices: Option<PathBuf>,

    // r[impl installer.dryrun.script]
    /// Path to a newline-delimited script file of key events to feed to the
    /// TUI instead of reading from the terminal.
    #[arg(long)]
    pub input_script: Option<PathBuf>,

    // r[impl installer.tui.timezone]
    /// Path to a text file of timezone names (one per line) for testing.
    /// When given, the installer reads timezones from this file instead of
    /// parsing /usr/share/zoneinfo/zone1970.tab.
    #[arg(long)]
    pub fake_timezones: Option<PathBuf>,

    // r[impl installer.dryrun.fake-tpm]
    /// Pretend a TPM device is present, regardless of whether /dev/tpm0 exists.
    #[arg(long)]
    pub fake_tpm: bool,

    // r[impl installer.no-reboot]
    /// Do not reboot after a successful installation. Exit cleanly instead.
    #[arg(long)]
    pub no_reboot: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if let Err(e) = init_logging(&cli.log) {
        eprintln!(
            "error: failed to initialize logging to {}: {e}",
            cli.log.display()
        );
        return ExitCode::FAILURE;
    }

    match run::RunContext::from_cli(cli).and_then(|ctx| ctx.run()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            // r[impl installer.container.error-logging]
            tracing::error!("{e:#}");
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn init_logging(log_path: &PathBuf) -> Result<()> {
    let file = File::create(log_path)
        .with_context(|| format!("creating log file {}", log_path.display()))?;
    let file_layer = fmt::layer()
        .with_writer(file)
        .with_ansi(false)
        .with_target(false);
    tracing_subscriber::registry().with(file_layer).init();
    Ok(())
}
