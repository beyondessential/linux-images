use std::fs::{self, File};
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;

mod config;
mod disk;
mod firstboot;
mod plan;
mod script;
mod ui;
mod writer;

const DEFAULT_LOG_PATH: &str = "/var/log/bes-installer.log";

#[derive(Parser)]
#[command(name = "bes-installer", about = "BES Linux Images Installer")]
struct Cli {
    /// Path to config file (overrides automatic EFI partition search)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Path to log file (default: /var/log/bes-installer.log)
    #[arg(long, default_value = DEFAULT_LOG_PATH)]
    log: PathBuf,

    // r[impl installer.dryrun]
    /// Dry-run mode: collect all decisions and emit an install plan as JSON
    /// instead of performing any destructive operations.
    #[arg(long)]
    dry_run: bool,

    // r[impl installer.dryrun.output]
    /// Path to write the dry-run JSON install plan. If omitted, the plan is
    /// written to stdout.
    #[arg(long)]
    dry_run_output: Option<PathBuf>,

    // r[impl installer.dryrun.devices]
    /// Path to a JSON file describing fake block devices (for testing).
    /// When given, the installer reads devices from this file instead of
    /// running lsblk.
    #[arg(long)]
    fake_devices: Option<PathBuf>,

    // r[impl installer.dryrun.script]
    /// Path to a newline-delimited script file of key events to feed to the
    /// TUI instead of reading from the terminal.
    #[arg(long)]
    input_script: Option<PathBuf>,

    // r[impl installer.no-reboot]
    /// Do not reboot after a successful installation. Exit cleanly instead.
    #[arg(long)]
    no_reboot: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(&cli.log);

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn init_logging(log_path: &PathBuf) {
    let file = File::create(log_path).ok();
    if let Some(file) = file {
        let file_layer = fmt::layer()
            .with_writer(file)
            .with_ansi(false)
            .with_target(false);
        tracing_subscriber::registry().with(file_layer).init();
    }
}

fn run(cli: Cli) -> Result<()> {
    let (install_config, mode) = load_config(&cli)?;

    let arch = detect_arch();
    tracing::info!("detected architecture: {arch}");

    // r[impl installer.dryrun.devices]
    let devices = if let Some(ref fake_path) = cli.fake_devices {
        tracing::info!("loading fake devices from {}", fake_path.display());
        disk::load_fake_devices(fake_path)?
    } else {
        disk::detect_block_devices().context("detecting block devices")?
    };

    if devices.is_empty() {
        bail!("no block devices found");
    }

    let boot_device = if cli.fake_devices.is_some() {
        None
    } else {
        disk::detect_boot_device()
    };
    if let Some(ref bd) = boot_device {
        tracing::info!("boot device: {}", bd.display());
    }

    let config_warnings = install_config.validate();
    for w in &config_warnings {
        tracing::warn!("config: {w}");
    }

    match mode {
        config::OperatingMode::Auto => run_auto(
            install_config,
            &arch,
            &devices,
            &boot_device,
            config_warnings,
            &cli,
        ),
        // r[impl installer.mode.auto-incomplete+2]
        config::OperatingMode::AutoIncomplete {
            missing_variant,
            missing_disk,
            missing_hostname,
        } => {
            let mut missing = Vec::new();
            if missing_variant {
                missing.push("variant");
            }
            if missing_disk {
                missing.push("disk");
            }
            if missing_hostname {
                missing.push("hostname (required for metal variant)");
            }
            eprintln!(
                "auto mode requested but required fields are missing: {}",
                missing.join(", ")
            );
            eprintln!("falling back to interactive mode");
            run_interactive(
                install_config,
                &mode,
                &arch,
                devices,
                boot_device,
                config_warnings,
                &cli,
            )
        }
        config::OperatingMode::Interactive | config::OperatingMode::Prefilled => run_interactive(
            install_config,
            &mode,
            &arch,
            devices,
            boot_device,
            config_warnings,
            &cli,
        ),
    }
}

fn load_config(cli: &Cli) -> Result<(config::InstallConfig, config::OperatingMode)> {
    let config_path = cli.config.clone().or_else(config::find_config_file);

    match config_path {
        Some(path) => {
            let cfg = config::InstallConfig::load_from_file(&path)?.unwrap_or_default();

            let mode = cfg.mode();
            tracing::info!("operating mode: {mode}");
            Ok((cfg, mode))
        }
        None => {
            tracing::info!("no config file found, using interactive mode");
            Ok((
                config::InstallConfig::default(),
                config::OperatingMode::Interactive,
            ))
        }
    }
}

// r[impl installer.mode.auto+2]
fn run_auto(
    cfg: config::InstallConfig,
    arch: &str,
    devices: &[disk::BlockDevice],
    boot_device: &Option<PathBuf>,
    config_warnings: Vec<String>,
    cli: &Cli,
) -> Result<()> {
    let variant = cfg.variant.expect("auto mode requires variant");
    let disk_selector = cfg.disk.as_ref().expect("auto mode requires disk");

    let target = disk::resolve_disk(disk_selector, devices, boot_device.as_ref())?;
    let image_path = if cli.dry_run {
        writer::find_image_path(&variant.to_string(), arch).ok()
    } else {
        Some(writer::find_image_path(&variant.to_string(), arch)?)
    };

    // r[impl installer.dryrun]
    if cli.dry_run {
        let plan = plan::InstallPlan::new(
            &config::OperatingMode::Auto,
            variant,
            Some(target),
            cfg.disable_tpm,
            cfg.firstboot.as_ref(),
            image_path,
            config_warnings,
        );
        return emit_plan(&plan, cli);
    }

    let image_path = image_path.unwrap();

    eprintln!("BES Installer -- automatic mode");
    eprintln!("  variant:    {variant}");
    eprintln!(
        "  target:     {} ({})",
        target.path.display(),
        target.size_display()
    );
    eprintln!("  image:      {}", image_path.display());
    eprintln!("  disable-tpm: {}", cfg.disable_tpm);

    if let Some(ref fb) = cfg.firstboot {
        if let Some(ref h) = fb.hostname {
            eprintln!("  hostname:   {h}");
        }
        if fb.tailscale_authkey.is_some() {
            eprintln!("  tailscale:  auth key provided");
        }
        if !fb.ssh_authorized_keys.is_empty() {
            eprintln!("  ssh keys:   {}", fb.ssh_authorized_keys.len());
        }
        if fb.has_password() {
            eprintln!("  password:   custom password set");
        }
    }

    // r[impl installer.write.disk-size-check]
    let image_size =
        writer::image_uncompressed_size(&image_path).context("reading uncompressed image size")?;
    writer::check_disk_size(image_size, target.size_bytes).context("disk size check")?;

    eprintln!();
    eprintln!("writing image...");

    writer::write_image(&image_path, &target.path, &mut |progress| {
        let pct = progress.fraction().map(|f| f * 100.0).unwrap_or(0.0);
        let mbps = progress.throughput_mbps();
        let eta = progress
            .eta()
            .map(writer::format_eta)
            .unwrap_or_else(|| "...".into());
        eprint!("\r  {pct:5.1}% | {mbps:.1} MiB/s | ETA: {eta}    ");
    })
    .context("writing image")?;
    eprintln!();

    writer::reread_partition_table(&target.path).context("re-reading partition table")?;
    writer::verify_partition_table(&target.path).context("verifying partition table")?;

    eprintln!("expanding partitions to fill disk...");
    writer::expand_partitions(&target.path).context("expanding partitions")?;

    if cfg.firstboot.is_some() || (variant == config::Variant::Metal && cfg.disable_tpm) {
        eprintln!("applying first-boot configuration...");
        let mounted = firstboot::mount_target(&target.path, variant)?;

        if let Some(ref fb) = cfg.firstboot {
            firstboot::apply_firstboot(&mounted, fb)?;
        }
        if variant == config::Variant::Metal && cfg.disable_tpm {
            firstboot::apply_tpm_disable(&mounted)?;
        }

        firstboot::unmount_target(mounted)?;
    }

    // r[impl installer.no-reboot]
    if cli.no_reboot {
        eprintln!("installation complete (--no-reboot, not rebooting)");
    } else {
        eprintln!("installation complete, rebooting...");
        let _ = std::process::Command::new("reboot").status();
    }
    Ok(())
}

// r[impl installer.mode.interactive]
// r[impl installer.mode.prefilled]
fn run_interactive(
    cfg: config::InstallConfig,
    mode: &config::OperatingMode,
    arch: &str,
    devices: Vec<disk::BlockDevice>,
    boot_device: Option<PathBuf>,
    config_warnings: Vec<String>,
    cli: &Cli,
) -> Result<()> {
    let variant = cfg.variant.unwrap_or(config::Variant::Metal);

    let default_disk_index = cfg.disk.as_ref().and_then(|sel| {
        disk::resolve_disk(sel, &devices, boot_device.as_ref())
            .ok()
            .and_then(|resolved| devices.iter().position(|d| d.path == resolved.path))
    });

    let image_path = if cli.dry_run {
        writer::find_image_path(&variant.to_string(), arch).ok()
    } else {
        Some(writer::find_image_path(&variant.to_string(), arch)?)
    };

    let build_info = read_build_info();

    let state = ui::AppState::new(
        devices,
        variant,
        cfg.disable_tpm,
        cfg.firstboot,
        boot_device,
        default_disk_index,
        build_info,
    );

    // r[impl installer.dryrun.script]
    // r[impl installer.dryrun.script.headless]
    if let Some(ref script_path) = cli.input_script {
        let events = script::parse_script_file(script_path)?;
        let final_state = ui::run_tui_scripted(state, events);

        if cli.dry_run {
            let plan = plan_from_tui_state(&final_state, mode, &image_path, &config_warnings);
            return emit_plan(&plan, cli);
        }

        eprintln!("scripted TUI finished on screen: {:?}", final_state.screen);
        return Ok(());
    }

    // r[impl installer.dryrun]
    if cli.dry_run {
        let plan = plan_from_tui_state(&state, mode, &image_path, &config_warnings);
        return emit_plan(&plan, cli);
    }

    let image_path = image_path.unwrap();
    ui::run_tui(state, &image_path, cli.no_reboot)
}

fn plan_from_tui_state(
    state: &ui::AppState,
    mode: &config::OperatingMode,
    image_path: &Option<PathBuf>,
    config_warnings: &[String],
) -> plan::InstallPlan {
    let disk = state.selected_disk();
    let firstboot = state.firstboot_config();
    plan::InstallPlan::new(
        mode,
        state.variant,
        disk,
        state.disable_tpm,
        firstboot.as_ref(),
        image_path.clone(),
        config_warnings.to_vec(),
    )
}

// r[impl installer.dryrun.output]
fn emit_plan(plan: &plan::InstallPlan, cli: &Cli) -> Result<()> {
    if let Some(ref path) = cli.dry_run_output {
        plan.write_to_path(path)?;
        tracing::info!("wrote install plan to {}", path.display());
    } else {
        plan.write_to_stdout()?;
    }
    Ok(())
}

fn read_build_info() -> String {
    let contents = match fs::read_to_string("/etc/bes-build-info") {
        Ok(c) => c,
        Err(_) => return String::new(),
    };

    let mut date = None;
    let mut arch = None;
    for line in contents.lines() {
        if let Some(val) = line.strip_prefix("BUILD_DATE=") {
            date = Some(val.trim());
        } else if let Some(val) = line.strip_prefix("ARCH=") {
            arch = Some(val.trim());
        }
    }

    match (date, arch) {
        (Some(d), Some(a)) => format!("Built {d} ({a})"),
        (Some(d), None) => format!("Built {d}"),
        _ => String::new(),
    }
}

fn detect_arch() -> String {
    let arch = std::env::consts::ARCH;
    match arch {
        "x86_64" => "amd64".into(),
        "aarch64" => "arm64".into(),
        other => other.into(),
    }
}
