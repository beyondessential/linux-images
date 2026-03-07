use std::fs;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Context, Result, bail};

use crate::Cli;
use crate::besconf;
use crate::config;
use crate::disk;
use crate::encryption;
use crate::firstboot;
use crate::hostname_template;
use crate::paths;
use crate::plan;
use crate::script;
use crate::timezone;
use crate::ui;
use crate::writer;

pub struct RunContext {
    pub cli: Cli,
    pub install_config: config::InstallConfig,
    pub mode: config::OperatingMode,
    pub devices: Vec<disk::BlockDevice>,
    pub boot_device: Option<PathBuf>,
    pub tpm_present: bool,
    #[expect(
        dead_code,
        reason = "logged at startup; will be used for arch-specific logic"
    )]
    pub arch: String,
    pub available_timezones: Vec<String>,
    pub config_warnings: Vec<String>,
    pub besconf: besconf::BesconfState,
}

impl RunContext {
    pub fn from_cli(cli: Cli) -> Result<Self> {
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

        // r[impl installer.config.recovery-passphrase]
        install_config.validate_hard()?;

        let config_warnings = install_config.validate();
        for w in &config_warnings {
            tracing::warn!("config: {w}");
        }

        // r[impl installer.besconf.writable-detection]
        let besconf = if cli.dry_run {
            besconf::BesconfState::readonly()
        } else {
            let state = besconf::detect_writable();
            // r[impl installer.besconf.failure-log]
            besconf::rotate_failure_log(&state);
            besconf::with_save_recovery_keys(state, install_config.save_recovery_keys)
        };

        Ok(Self {
            cli,
            install_config,
            mode,
            devices,
            boot_device,
            tpm_present,
            arch,
            available_timezones,
            config_warnings,
            besconf,
        })
    }

    pub fn run(self) -> Result<()> {
        let besconf = self.besconf.clone();
        let log_path = self.cli.log.clone();
        let result = self.run_inner();

        // r[impl installer.besconf.failure-log]
        if result.is_err() {
            besconf::write_failure_log(&besconf, &log_path);
        }

        result
    }

    fn run_inner(self) -> Result<()> {
        match self.mode {
            config::OperatingMode::Auto => self.run_auto(),
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
                self.run_interactive()
            }
            config::OperatingMode::Interactive | config::OperatingMode::Prefilled => {
                self.run_interactive()
            }
        }
    }

    // r[impl installer.mode.auto+4]
    fn run_auto(self) -> Result<()> {
        let disk_encryption = self
            .install_config
            .disk_encryption
            .expect("auto mode requires disk-encryption");
        let variant = disk_encryption.variant();
        let disk_selector = self
            .install_config
            .disk
            .as_ref()
            .expect("auto mode requires disk");

        let copy_install_log = self.install_config.copy_install_log.unwrap_or(true);

        let hostname_from_template = self.install_config.hostname_template.is_some();

        let target = disk::resolve_disk(disk_selector, &self.devices, self.boot_device.as_ref())?;
        let manifest_result = if self.cli.dry_run {
            writer::find_partition_manifest().ok()
        } else {
            Some(writer::find_partition_manifest()?)
        };
        let manifest_path = manifest_result
            .as_ref()
            .map(|(_, dir)| dir.join("partitions.json"));

        let effective_timezone = self.install_config.timezone.as_deref().unwrap_or("UTC");

        // r[impl installer.dryrun]
        if self.cli.dry_run {
            let mut builder =
                plan::InstallPlan::builder(&config::OperatingMode::Auto, disk_encryption)
                    .disk(target)
                    .tpm_present(self.tpm_present)
                    .hostname_from_template(hostname_from_template)
                    .timezone(effective_timezone)
                    .copy_install_log(copy_install_log)
                    .save_recovery_keys(self.besconf.save_recovery_keys())
                    .config_warnings(self.config_warnings);
            if self.install_config.has_install_config_fields() {
                builder = builder.install_config(&self.install_config);
            }
            if let Some(path) = manifest_path {
                builder = builder.manifest_path(path);
            }
            let plan = builder.build();
            return emit_plan(&plan, &self.cli);
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
        eprintln!("  tpm:        {}", self.tpm_present);

        if self.install_config.hostname_from_dhcp {
            eprintln!("  hostname:   (from DHCP)");
        } else if let Some(ref h) = self.install_config.hostname {
            eprintln!("  hostname:   {h}");
        }
        if let Some(ref tz) = self.install_config.timezone {
            eprintln!("  timezone:   {tz}");
        } else {
            eprintln!("  timezone:   UTC");
        }
        if self.install_config.tailscale_authkey.is_some() {
            eprintln!("  tailscale:  auth key provided");
        }
        if !self.install_config.ssh_authorized_keys.is_empty() {
            eprintln!(
                "  ssh keys:   {}",
                self.install_config.ssh_authorized_keys.len()
            );
        }
        if self.install_config.has_password() {
            eprintln!("  password:   custom password set");
        }

        // r[impl installer.write.disk-size-check+3]
        let total_image_size = writer::partition_images_total_size(&manifest, &images_dir)
            .context("reading partition image sizes")?;
        writer::check_disk_size(total_image_size, target.size_bytes).context("disk size check")?;

        // r[impl installer.encryption.recovery-passphrase+3]
        // r[impl installer.config.recovery-passphrase]
        let recovery_passphrase = if disk_encryption.is_encrypted() {
            Some(
                self.install_config
                    .recovery_passphrase
                    .clone()
                    .unwrap_or_else(encryption::generate_recovery_passphrase),
            )
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

        // r[impl installer.encryption.overview+3]
        if let Some(ref passphrase) = recovery_passphrase {
            eprintln!("setting up disk encryption...");
            let mounted = firstboot::mount_target(
                &target.path,
                disk_encryption,
                recovery_passphrase.as_deref(),
            )?;
            encryption::enroll_and_configure_encryption(
                &target.path,
                disk_encryption,
                mounted.path(),
                passphrase,
            )
            .context("encryption setup")?;
            firstboot::unmount_target(mounted)?;
        }

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
                firstboot::fixup_for_metal_variant(&mounted, &self.install_config)?;
            }

            if self.install_config.has_install_config_fields() {
                firstboot::apply_firstboot(&mounted, &self.install_config, false)?;
            } else {
                // r[impl installer.finalise.timezone]
                firstboot::apply_timezone_default(&mounted)?;
            }

            // r[impl installer.finalise.copy-install-log+2]
            if copy_install_log {
                firstboot::copy_install_log(&mounted, &self.cli.log);
            }

            firstboot::unmount_target(mounted)?;
        }

        if let Some(ref passphrase) = recovery_passphrase {
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

            // r[impl installer.config.save-recovery-keys]
            if self.besconf.save_recovery_keys() {
                let root_part = crate::util::partition_path(&target.path, 3)?;
                if let Err(e) = besconf::append_recovery_key(&self.besconf, passphrase, &root_part)
                {
                    tracing::warn!("failed to save recovery key to BESCONF: {e}");
                    eprintln!("warning: failed to save recovery key to BESCONF: {e}");
                }
            }
        }

        // r[impl installer.no-reboot]
        if self.cli.no_reboot {
            eprintln!("installation complete (--no-reboot, not rebooting)");
        } else {
            eprintln!("installation complete, rebooting...");
            let reboot_ok = std::process::Command::new(paths::REBOOT)
                .status()
                .is_ok_and(|s| s.success());
            if !reboot_ok {
                tracing::warn!("{} failed, trying systemctl", paths::REBOOT);
                let _ = std::process::Command::new(paths::SYSTEMCTL)
                    .arg("reboot")
                    .status();
            }
        }
        Ok(())
    }

    // r[impl installer.mode.interactive+2]
    // r[impl installer.mode.prefilled]
    fn run_interactive(self) -> Result<()> {
        let disk_encryption = self
            .install_config
            .disk_encryption
            .unwrap_or(if self.tpm_present {
                config::DiskEncryption::Tpm
            } else {
                config::DiskEncryption::Keyfile
            });

        let copy_install_log = self.install_config.copy_install_log.unwrap_or(true);

        let default_disk_index = self.install_config.disk.as_ref().and_then(|sel| {
            disk::resolve_disk(sel, &self.devices, self.boot_device.as_ref())
                .ok()
                .and_then(|resolved| self.devices.iter().position(|d| d.path == resolved.path))
        });

        let manifest_result = if self.cli.dry_run {
            writer::find_partition_manifest().ok()
        } else {
            Some(writer::find_partition_manifest()?)
        };
        let manifest_path = manifest_result
            .as_ref()
            .map(|(_, dir)| dir.join("partitions.json"));

        let build_info = read_build_info();

        let hostname_from_template = self.install_config.hostname_template.is_some();

        let mut state = ui::AppState::new(
            self.devices,
            disk_encryption,
            self.tpm_present,
            &self.install_config,
            self.boot_device,
            default_disk_index,
            build_info,
            self.available_timezones,
        );

        // r[impl installer.config.recovery-passphrase]
        if let Some(ref pp) = self.install_config.recovery_passphrase {
            state.recovery_passphrase = Some(pp.clone());
        }

        // r[impl installer.dryrun.script]
        // r[impl installer.dryrun.script.headless]
        if let Some(ref script_path) = self.cli.input_script {
            let events = script::parse_script_file(script_path)?;
            let final_state = ui::run_tui_scripted(state, events);

            if self.cli.dry_run {
                let plan = plan_from_tui_state(
                    &final_state,
                    &self.mode,
                    hostname_from_template,
                    &manifest_path,
                    copy_install_log,
                    self.besconf.save_recovery_keys(),
                    &self.config_warnings,
                );
                return emit_plan(&plan, &self.cli);
            }

            eprintln!("scripted TUI finished on screen: {:?}", final_state.screen);
            return Ok(());
        }

        // r[impl installer.dryrun]
        if self.cli.dry_run {
            let plan = plan_from_tui_state(
                &state,
                &self.mode,
                hostname_from_template,
                &manifest_path,
                copy_install_log,
                self.besconf.save_recovery_keys(),
                &self.config_warnings,
            );
            return emit_plan(&plan, &self.cli);
        }

        let (manifest, images_dir) = manifest_result.unwrap();
        let install_log = if copy_install_log {
            Some(self.cli.log.as_path())
        } else {
            None
        };
        ui::run_tui(
            state,
            &manifest,
            &images_dir,
            install_log,
            self.cli.no_reboot,
            &self.besconf,
        )
    }
}

fn resolve_hostname_template(cfg: &mut config::InstallConfig) -> Result<()> {
    let Some(ref tmpl) = cfg.hostname_template.clone() else {
        return Ok(());
    };
    let resolved = hostname_template::parse_and_resolve(tmpl)
        .with_context(|| format!("resolving hostname template '{tmpl}'"))?;
    tracing::info!("resolved hostname template '{tmpl}' to '{resolved}'");
    cfg.hostname = Some(resolved);
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

fn plan_from_tui_state(
    state: &ui::AppState,
    mode: &config::OperatingMode,
    hostname_from_template: bool,
    manifest_path: &Option<PathBuf>,
    copy_install_log: bool,
    save_recovery_keys: bool,
    config_warnings: &[String],
) -> plan::InstallPlan {
    let disk = state.selected_disk();
    let install_cfg = state.install_config_fields();
    let mut builder = plan::InstallPlan::builder(mode, state.disk_encryption)
        .tpm_present(state.tpm_present)
        .hostname_from_template(state.hostname_from_template || hostname_from_template)
        .timezone(state.effective_timezone())
        .copy_install_log(copy_install_log)
        .save_recovery_keys(save_recovery_keys)
        .config_warnings(config_warnings.to_vec());
    if let Some(dev) = disk {
        builder = builder.disk(dev);
    }
    if let Some(ref cfg) = install_cfg {
        builder = builder.install_config(cfg);
    }
    if let Some(path) = manifest_path {
        builder = builder.manifest_path(path.clone());
    }
    builder.build()
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
    let commit = crate::Bosion::GIT_COMMIT_SHORTHASH;
    let bosion_date = crate::Bosion::BUILD_DATE;

    let (date, arch) = match fs::read_to_string("/etc/bes-build-info") {
        Ok(contents) => {
            let mut d = None;
            let mut a = None;
            for line in contents.lines() {
                if let Some(val) = line.strip_prefix("BUILD_DATE=") {
                    d = Some(val.trim().to_string());
                } else if let Some(val) = line.strip_prefix("ARCH=") {
                    a = Some(val.trim().to_string());
                }
            }
            (d, a)
        }
        Err(_) => (None, None),
    };

    let date = date.unwrap_or_else(|| bosion_date.to_string());
    let arch = arch.unwrap_or_else(detect_arch);

    if commit.is_empty() {
        format!("Built {date} ({arch})")
    } else {
        format!("Built {date} ({arch}) {commit}")
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
