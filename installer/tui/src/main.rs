use std::fs::{self, File};
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{Context, Result, bail};
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
mod script;
mod timezone;
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

    // r[impl installer.tui.timezone]
    /// Path to a text file of timezone names (one per line) for testing.
    /// When given, the installer reads timezones from this file instead of
    /// parsing /usr/share/zoneinfo/zone1970.tab.
    #[arg(long)]
    fake_timezones: Option<PathBuf>,

    // r[impl installer.dryrun.fake-tpm]
    /// Pretend a TPM device is present, regardless of whether /dev/tpm0 exists.
    #[arg(long)]
    fake_tpm: bool,

    // r[impl installer.no-reboot]
    /// Do not reboot after a successful installation. Exit cleanly instead.
    #[arg(long)]
    no_reboot: bool,
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

    match run(cli) {
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

fn run(cli: Cli) -> Result<()> {
    let (mut install_config, mode) = load_config(&cli)?;

    resolve_hostname_template(&mut install_config)?;

    // r[impl installer.tui.timezone]
    let available_timezones = if let Some(ref fake_path) = cli.fake_timezones {
        tracing::info!("loading fake timezones from {}", fake_path.display());
        timezone::load_from_file(fake_path)?
    } else {
        timezone::load_system_timezones()
    };
    tracing::info!("loaded {} timezones", available_timezones.len());

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

    // r[impl installer.dryrun.fake-tpm]
    let tpm_present = if cli.fake_devices.is_some() {
        cli.fake_tpm
    } else {
        cli.fake_tpm || std::path::Path::new("/dev/tpm0").exists()
    };
    tracing::info!("TPM present: {tpm_present}");

    let config_warnings = install_config.validate();
    for w in &config_warnings {
        tracing::warn!("config: {w}");
    }

    match mode {
        config::OperatingMode::Auto => run_auto(
            install_config,
            tpm_present,
            &arch,
            &devices,
            &boot_device,
            config_warnings,
            &cli,
        ),
        // r[impl installer.mode.auto-incomplete+3]
        config::OperatingMode::AutoIncomplete {
            missing_disk_encryption,
            missing_disk,
            missing_hostname,
        } => {
            let mut missing = Vec::new();
            if missing_disk_encryption {
                missing.push("disk-encryption");
            }
            if missing_disk {
                missing.push("disk");
            }
            if missing_hostname {
                missing.push("hostname strategy (hostname, hostname-from-dhcp, or hostname-template required for encrypted variants)");
            }
            eprintln!(
                "auto mode requested but required fields are missing: {}",
                missing.join(", ")
            );
            eprintln!("falling back to interactive mode");
            run_interactive(
                install_config,
                &mode,
                tpm_present,
                &arch,
                devices,
                boot_device,
                available_timezones,
                config_warnings,
                &cli,
            )
        }
        config::OperatingMode::Interactive | config::OperatingMode::Prefilled => run_interactive(
            install_config,
            &mode,
            tpm_present,
            &arch,
            devices,
            boot_device,
            available_timezones,
            config_warnings,
            &cli,
        ),
    }
}

fn resolve_hostname_template(cfg: &mut config::InstallConfig) -> Result<()> {
    let Some(ref mut fb) = cfg.firstboot else {
        return Ok(());
    };
    let Some(ref tmpl) = fb.hostname_template.clone() else {
        return Ok(());
    };
    let resolved = hostname_template::parse_and_resolve(tmpl)
        .with_context(|| format!("resolving hostname template '{tmpl}'"))?;
    tracing::info!("resolved hostname template '{tmpl}' to '{resolved}'");
    fb.hostname = Some(resolved);
    Ok(())
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

// r[impl installer.mode.auto+3]
fn run_auto(
    cfg: config::InstallConfig,
    tpm_present: bool,
    _arch: &str,
    devices: &[disk::BlockDevice],
    boot_device: &Option<PathBuf>,
    config_warnings: Vec<String>,
    cli: &Cli,
) -> Result<()> {
    let disk_encryption = cfg
        .disk_encryption
        .expect("auto mode requires disk-encryption");
    let variant = disk_encryption.variant();
    let disk_selector = cfg.disk.as_ref().expect("auto mode requires disk");

    let copy_install_log = cfg.copy_install_log.unwrap_or(true);

    let hostname_from_template = cfg
        .firstboot
        .as_ref()
        .is_some_and(|fb| fb.hostname_template.is_some());

    let target = disk::resolve_disk(disk_selector, devices, boot_device.as_ref())?;
    let manifest_result = if cli.dry_run {
        writer::find_partition_manifest().ok()
    } else {
        Some(writer::find_partition_manifest()?)
    };
    let manifest_path = manifest_result
        .as_ref()
        .map(|(_, dir)| dir.join("partitions.json"));

    let effective_timezone = cfg
        .firstboot
        .as_ref()
        .and_then(|fb| fb.timezone.as_deref())
        .unwrap_or("UTC");

    // r[impl installer.dryrun]
    if cli.dry_run {
        let plan = plan::InstallPlan::new(
            &config::OperatingMode::Auto,
            disk_encryption,
            Some(target),
            tpm_present,
            cfg.firstboot.as_ref(),
            hostname_from_template,
            effective_timezone,
            manifest_path,
            copy_install_log,
            config_warnings,
        );
        return emit_plan(&plan, cli);
    }

    let (manifest, images_dir) = manifest_result.unwrap();

    eprintln!("BES Installer -- automatic mode");
    eprintln!("  encryption: {disk_encryption}");
    eprintln!("  variant:    {variant}");
    eprintln!(
        "  target:     {} ({})",
        target.path.display(),
        target.size_display()
    );
    eprintln!(
        "  manifest:   {}",
        images_dir.join("partitions.json").display()
    );
    eprintln!("  tpm:        {tpm_present}");

    if let Some(ref fb) = cfg.firstboot {
        if fb.hostname_from_dhcp {
            eprintln!("  hostname:   (from DHCP)");
        } else if let Some(ref h) = fb.hostname {
            eprintln!("  hostname:   {h}");
        }
        if let Some(ref tz) = fb.timezone {
            eprintln!("  timezone:   {tz}");
        } else {
            eprintln!("  timezone:   UTC");
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

    // r[impl installer.write.disk-size-check+2]
    let total_image_size = writer::partition_images_total_size(&manifest, &images_dir)
        .context("reading partition image sizes")?;
    writer::check_disk_size(total_image_size, target.size_bytes).context("disk size check")?;

    // r[impl installer.encryption.recovery-passphrase+3]
    let recovery_passphrase = if disk_encryption.is_encrypted() {
        Some(encryption::generate_recovery_passphrase())
    } else {
        None
    };

    let disk_writer = writer::DiskWriter::new(
        &target.path,
        disk_encryption,
        recovery_passphrase.as_deref(),
    );

    eprintln!();
    eprintln!("writing partitions...");

    // r[impl installer.mode.auto.progress]
    let interactive = std::io::stderr().is_terminal();
    let write_start = Instant::now();
    disk_writer
        .write_partitions(&manifest, &images_dir, &mut |progress| {
            if interactive {
                let pct = progress.fraction().map(|f| f * 100.0).unwrap_or(0.0);
                let mbps = progress.throughput_mbps();
                let eta = progress
                    .eta()
                    .map(writer::format_eta)
                    .unwrap_or_else(|| "...".into());
                eprint!("\r  {pct:5.1}% | {mbps:.1} MiB/s | ETA: {eta}    ");
            }
        })
        .context("writing partitions")?;

    // r[impl installer.mode.auto.progress]
    if interactive {
        eprintln!();
    } else {
        let size_mib = total_image_size as f64 / (1024.0 * 1024.0);
        let secs = write_start.elapsed().as_secs_f64();
        let mbps = if secs > 0.0 { size_mib / secs } else { 0.0 };
        eprintln!("write complete: {size_mib:.1} MiB in {secs:.1}s ({mbps:.1} MiB/s)");
    }

    eprintln!("expanding root filesystem...");
    disk_writer
        .expand_root_filesystem()
        .context("expanding root filesystem")?;

    eprintln!("randomizing filesystem UUIDs...");
    disk_writer
        .randomize_filesystem_uuids()
        .context("randomizing filesystem UUIDs")?;

    eprintln!("rebuilding boot config (initramfs + grub)...");
    disk_writer
        .rebuild_boot_config()
        .context("rebuilding boot config")?;

    disk_writer
        .verify_partition_table()
        .context("verifying partition table")?;

    {
        eprintln!("applying first-boot configuration...");
        let mounted = firstboot::mount_target(
            &target.path,
            disk_encryption,
            recovery_passphrase.as_deref(),
        )?;

        // r[impl installer.write.fstab-fixup]
        // r[impl installer.write.variant-fixup]
        if disk_encryption.is_encrypted() {
            firstboot::fixup_for_metal_variant(&mounted, &cfg.firstboot)?;
        }

        if let Some(ref fb) = cfg.firstboot {
            firstboot::apply_firstboot(&mounted, fb)?;
        } else {
            // r[impl installer.firstboot.timezone]
            firstboot::apply_timezone_default(&mounted)?;
        }

        // r[impl installer.firstboot.copy-install-log]
        if copy_install_log {
            firstboot::copy_install_log(&mounted, &cli.log);
        }

        firstboot::unmount_target(mounted)?;
    }

    // r[impl installer.encryption.overview+2]
    if let Some(ref passphrase) = recovery_passphrase {
        eprintln!("setting up disk encryption...");
        let mounted = firstboot::mount_target(
            &target.path,
            disk_encryption,
            recovery_passphrase.as_deref(),
        )?;
        encryption::run_encryption_setup(&target.path, disk_encryption, mounted.path(), passphrase)
            .context("encryption setup")?;
        firstboot::unmount_target(mounted)?;

        // r[impl installer.encryption.recovery-passphrase+3]
        eprintln!();
        eprintln!("=== RECOVERY PASSPHRASE ===");
        eprintln!();
        eprintln!("  {passphrase}");
        eprintln!();
        eprintln!("Write down this passphrase and store it in a safe place.");
        eprintln!("You will need it if the primary unlock mechanism fails.");
        eprintln!("===========================");
        eprintln!();
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

// r[impl installer.mode.interactive+2]
// r[impl installer.mode.prefilled]
#[expect(
    clippy::too_many_arguments,
    reason = "collecting all state needed to launch the interactive TUI"
)]
fn run_interactive(
    cfg: config::InstallConfig,
    mode: &config::OperatingMode,
    tpm_present: bool,
    _arch: &str,
    devices: Vec<disk::BlockDevice>,
    boot_device: Option<PathBuf>,
    available_timezones: Vec<String>,
    config_warnings: Vec<String>,
    cli: &Cli,
) -> Result<()> {
    let disk_encryption = cfg.disk_encryption.unwrap_or(if tpm_present {
        config::DiskEncryption::Tpm
    } else {
        config::DiskEncryption::Keyfile
    });

    let copy_install_log = cfg.copy_install_log.unwrap_or(true);

    let default_disk_index = cfg.disk.as_ref().and_then(|sel| {
        disk::resolve_disk(sel, &devices, boot_device.as_ref())
            .ok()
            .and_then(|resolved| devices.iter().position(|d| d.path == resolved.path))
    });

    let manifest_result = if cli.dry_run {
        writer::find_partition_manifest().ok()
    } else {
        Some(writer::find_partition_manifest()?)
    };
    let manifest_path = manifest_result
        .as_ref()
        .map(|(_, dir)| dir.join("partitions.json"));

    let build_info = read_build_info();

    let hostname_from_template = cfg
        .firstboot
        .as_ref()
        .is_some_and(|fb| fb.hostname_template.is_some());

    let state = ui::AppState::new(
        devices,
        disk_encryption,
        tpm_present,
        cfg.firstboot,
        boot_device,
        default_disk_index,
        build_info,
        available_timezones,
    );

    // r[impl installer.dryrun.script]
    // r[impl installer.dryrun.script.headless]
    if let Some(ref script_path) = cli.input_script {
        let events = script::parse_script_file(script_path)?;
        let final_state = ui::run_tui_scripted(state, events);

        if cli.dry_run {
            let plan = plan_from_tui_state(
                &final_state,
                mode,
                hostname_from_template,
                &manifest_path,
                copy_install_log,
                &config_warnings,
            );
            return emit_plan(&plan, cli);
        }

        eprintln!("scripted TUI finished on screen: {:?}", final_state.screen);
        return Ok(());
    }

    // r[impl installer.dryrun]
    if cli.dry_run {
        let plan = plan_from_tui_state(
            &state,
            mode,
            hostname_from_template,
            &manifest_path,
            copy_install_log,
            &config_warnings,
        );
        return emit_plan(&plan, cli);
    }

    let (manifest, images_dir) = manifest_result.unwrap();
    ui::run_tui(
        state,
        &manifest,
        &images_dir,
        copy_install_log,
        &cli.log,
        cli.no_reboot,
    )
}

fn plan_from_tui_state(
    state: &ui::AppState,
    mode: &config::OperatingMode,
    hostname_from_template: bool,
    manifest_path: &Option<PathBuf>,
    copy_install_log: bool,
    config_warnings: &[String],
) -> plan::InstallPlan {
    let disk = state.selected_disk();
    let firstboot = state.firstboot_config();
    plan::InstallPlan::new(
        mode,
        state.disk_encryption,
        disk,
        state.tpm_present,
        firstboot.as_ref(),
        state.hostname_from_template || hostname_from_template,
        state.effective_timezone(),
        manifest_path.clone(),
        copy_install_log,
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
