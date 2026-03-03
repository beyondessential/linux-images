// r[impl installer.mode.interactive]
// r[impl installer.mode.prefilled]
// r[impl installer.mode.auto]
// r[impl installer.mode.auto-incomplete]

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::Parser;

mod config;
mod disk;
mod firstboot;
mod ui;
mod writer;

#[derive(Parser)]
#[command(name = "bes-installer", about = "BES Linux Images Installer")]
struct Cli {
    /// Path to config file (overrides automatic EFI partition search)
    #[arg(long)]
    config: Option<PathBuf>,
}

fn main() -> ExitCode {
    env_logger::init();
    let cli = Cli::parse();

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let (install_config, mode) = load_config(&cli)?;

    let arch = detect_arch();
    log::info!("detected architecture: {arch}");

    let devices = disk::detect_block_devices().context("detecting block devices")?;
    if devices.is_empty() {
        bail!("no block devices found");
    }

    let boot_device = disk::detect_boot_device();
    if let Some(ref bd) = boot_device {
        log::info!("boot device: {}", bd.display());
    }

    match mode {
        config::OperatingMode::Auto => run_auto(install_config, &arch, &devices, &boot_device),
        config::OperatingMode::AutoIncomplete {
            missing_variant,
            missing_disk,
        } => {
            let mut missing = Vec::new();
            if missing_variant {
                missing.push("variant");
            }
            if missing_disk {
                missing.push("disk");
            }
            eprintln!(
                "auto mode requested but required fields are missing: {}",
                missing.join(", ")
            );
            eprintln!("falling back to interactive mode");
            run_interactive(install_config, &arch, devices, boot_device)
        }
        config::OperatingMode::Interactive | config::OperatingMode::Prefilled => {
            run_interactive(install_config, &arch, devices, boot_device)
        }
    }
}

fn load_config(cli: &Cli) -> Result<(config::InstallConfig, config::OperatingMode)> {
    let config_path = cli.config.clone().or_else(config::find_config_file);

    match config_path {
        Some(path) => {
            let cfg = config::InstallConfig::load_from_file(&path)?.unwrap_or_default();

            let warnings = cfg.validate();
            for w in &warnings {
                log::warn!("config: {w}");
            }

            let mode = cfg.mode();
            log::info!("operating mode: {mode}");
            Ok((cfg, mode))
        }
        None => {
            log::info!("no config file found, using interactive mode");
            Ok((
                config::InstallConfig::default(),
                config::OperatingMode::Interactive,
            ))
        }
    }
}

fn run_auto(
    cfg: config::InstallConfig,
    arch: &str,
    devices: &[disk::BlockDevice],
    boot_device: &Option<PathBuf>,
) -> Result<()> {
    let variant = cfg.variant.expect("auto mode requires variant");
    let disk_selector = cfg.disk.as_ref().expect("auto mode requires disk");

    let target = disk::resolve_disk(disk_selector, devices, boot_device.as_ref())?;
    let image_path = writer::find_image_path(&variant.to_string(), arch)?;

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
    }

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

    eprintln!("installation complete, rebooting...");
    let _ = std::process::Command::new("reboot").status();
    Ok(())
}

fn run_interactive(
    cfg: config::InstallConfig,
    arch: &str,
    devices: Vec<disk::BlockDevice>,
    boot_device: Option<PathBuf>,
) -> Result<()> {
    let variant = cfg.variant.unwrap_or(config::Variant::Metal);

    let default_disk_index = cfg.disk.as_ref().and_then(|sel| {
        disk::resolve_disk(sel, &devices, boot_device.as_ref())
            .ok()
            .and_then(|resolved| devices.iter().position(|d| d.path == resolved.path))
    });

    let image_path = writer::find_image_path(&variant.to_string(), arch)?;

    let state = ui::AppState::new(
        devices,
        variant,
        cfg.disable_tpm,
        cfg.firstboot,
        boot_device,
        default_disk_index,
    );

    ui::run_tui(state, &image_path)
}

fn detect_arch() -> String {
    let arch = std::env::consts::ARCH;
    match arch {
        "x86_64" => "amd64".into(),
        "aarch64" => "arm64".into(),
        other => other.into(),
    }
}
