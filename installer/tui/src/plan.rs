use std::path::PathBuf;

use serde::Serialize;

use crate::config::{DiskEncryption, InstallConfig, OperatingMode};
use crate::disk::BlockDevice;

// r[impl installer.dryrun.schema+5]
#[derive(Debug, Clone, Serialize)]
pub struct InstallPlan {
    pub mode: String,
    pub disk_encryption: String,
    pub variant: String,
    pub disk: Option<DiskInfo>,
    pub tpm_present: bool,
    pub install_config: Option<InstallConfigInfo>,
    pub manifest_path: Option<PathBuf>,
    pub copy_install_log: bool,
    pub save_recovery_keys: bool,
    pub config_warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskInfo {
    pub path: PathBuf,
    pub model: String,
    pub size_bytes: u64,
    pub transport: String,
}

// r[impl installer.dryrun.schema+5]
#[derive(Debug, Clone, Serialize)]
pub struct InstallConfigInfo {
    pub hostname: Option<String>,
    pub hostname_from_template: bool,
    pub tailscale_authkey: bool,
    pub ssh_authorized_keys_count: usize,
    pub password_set: bool,
    pub timezone: String,
}

impl From<&BlockDevice> for DiskInfo {
    fn from(dev: &BlockDevice) -> Self {
        DiskInfo {
            path: dev.path.clone(),
            model: dev.model.clone(),
            size_bytes: dev.size_bytes,
            transport: dev.transport.to_string(),
        }
    }
}

impl InstallConfigInfo {
    pub fn from_config(cfg: &InstallConfig, hostname_from_template: bool, timezone: &str) -> Self {
        let hostname = if cfg.hostname_from_dhcp {
            Some("dhcp".to_string())
        } else {
            cfg.hostname.clone()
        };
        InstallConfigInfo {
            hostname,
            hostname_from_template,
            tailscale_authkey: cfg.tailscale_authkey.is_some(),
            ssh_authorized_keys_count: cfg.ssh_authorized_keys.len(),
            password_set: cfg.has_password(),
            timezone: timezone.to_string(),
        }
    }
}

/// Builder for [`InstallPlan`].
///
/// Only `mode` and `disk_encryption` are required. Everything else has a
/// sensible default (`false`, `None`, `"UTC"`, `true` for copy_install_log,
/// empty warnings).
pub struct InstallPlanBuilder<'a> {
    mode: &'a OperatingMode,
    disk_encryption: DiskEncryption,
    disk: Option<&'a BlockDevice>,
    tpm_present: bool,
    install_config: Option<&'a InstallConfig>,
    hostname_from_template: bool,
    timezone: &'a str,
    manifest_path: Option<PathBuf>,
    copy_install_log: bool,
    save_recovery_keys: bool,
    config_warnings: Vec<String>,
}

impl<'a> InstallPlanBuilder<'a> {
    pub fn new(mode: &'a OperatingMode, disk_encryption: DiskEncryption) -> Self {
        Self {
            mode,
            disk_encryption,
            disk: None,
            tpm_present: false,
            install_config: None,
            hostname_from_template: false,
            timezone: "UTC",
            manifest_path: None,
            copy_install_log: true,
            save_recovery_keys: false,
            config_warnings: Vec::new(),
        }
    }

    pub fn disk(mut self, disk: &'a BlockDevice) -> Self {
        self.disk = Some(disk);
        self
    }

    pub fn tpm_present(mut self, present: bool) -> Self {
        self.tpm_present = present;
        self
    }

    pub fn install_config(mut self, cfg: &'a InstallConfig) -> Self {
        self.install_config = Some(cfg);
        self
    }

    pub fn hostname_from_template(mut self, from_template: bool) -> Self {
        self.hostname_from_template = from_template;
        self
    }

    pub fn timezone(mut self, tz: &'a str) -> Self {
        self.timezone = tz;
        self
    }

    pub fn manifest_path(mut self, path: PathBuf) -> Self {
        self.manifest_path = Some(path);
        self
    }

    pub fn copy_install_log(mut self, copy: bool) -> Self {
        self.copy_install_log = copy;
        self
    }

    pub fn save_recovery_keys(mut self, save: bool) -> Self {
        self.save_recovery_keys = save;
        self
    }

    pub fn config_warnings(mut self, warnings: Vec<String>) -> Self {
        self.config_warnings = warnings;
        self
    }

    pub fn build(self) -> InstallPlan {
        let mode_str = match self.mode {
            OperatingMode::Interactive => "interactive",
            OperatingMode::Prefilled => "prefilled",
            OperatingMode::Auto => "auto",
            OperatingMode::AutoIncomplete { .. } => "auto-incomplete",
        };

        InstallPlan {
            mode: mode_str.to_string(),
            disk_encryption: self.disk_encryption.to_string(),
            variant: self.disk_encryption.variant().to_string(),
            disk: self.disk.map(DiskInfo::from),
            tpm_present: self.tpm_present,
            install_config: self.install_config.map(|cfg| {
                InstallConfigInfo::from_config(cfg, self.hostname_from_template, self.timezone)
            }),
            manifest_path: self.manifest_path,
            copy_install_log: self.copy_install_log,
            save_recovery_keys: self.save_recovery_keys,
            config_warnings: self.config_warnings,
        }
    }
}

impl InstallPlan {
    pub fn builder<'a>(
        mode: &'a OperatingMode,
        disk_encryption: DiskEncryption,
    ) -> InstallPlanBuilder<'a> {
        InstallPlanBuilder::new(mode, disk_encryption)
    }

    pub fn write_to_path(&self, path: &PathBuf) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn write_to_stdout(&self) -> anyhow::Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        println!("{json}");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::config::{DiskEncryption, InstallConfig, OperatingMode};
    use crate::disk::{BlockDevice, TransportType};

    fn sample_device() -> BlockDevice {
        BlockDevice {
            path: PathBuf::from("/dev/nvme0n1"),
            size_bytes: 1_000_204_886_016,
            model: "Samsung 980 PRO".into(),
            transport: TransportType::Nvme,
            removable: false,
        }
    }

    fn sample_install_config() -> InstallConfig {
        InstallConfig {
            hostname: Some("server-01".into()),
            tailscale_authkey: Some("tskey-auth-xxxxx".into()),
            ssh_authorized_keys: vec!["ssh-ed25519 AAAA key1".into(), "ssh-rsa BBBB key2".into()],
            password: Some("changeme".into()),
            ..Default::default()
        }
    }

    // r[verify installer.dryrun.schema+5]
    #[test]
    fn plan_serializes_full() {
        let dev = sample_device();
        let cfg = sample_install_config();
        let plan = InstallPlan::builder(&OperatingMode::Auto, DiskEncryption::Tpm)
            .disk(&dev)
            .tpm_present(true)
            .install_config(&cfg)
            .timezone("America/New_York")
            .manifest_path(PathBuf::from("/run/live/medium/images/partitions.json"))
            .build();

        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(json["mode"], "auto");
        assert_eq!(json["disk_encryption"], "tpm");
        assert_eq!(json["variant"], "metal");
        assert_eq!(json["disk"]["path"], "/dev/nvme0n1");
        assert_eq!(json["disk"]["model"], "Samsung 980 PRO");
        assert_eq!(json["disk"]["size_bytes"], 1_000_204_886_016u64);
        assert_eq!(json["disk"]["transport"], "NVMe");
        assert!(json["tpm_present"].as_bool().unwrap());
        assert_eq!(json["install_config"]["hostname"], "server-01");
        assert!(
            !json["install_config"]["hostname_from_template"]
                .as_bool()
                .unwrap()
        );
        assert!(
            json["install_config"]["tailscale_authkey"]
                .as_bool()
                .unwrap()
        );
        assert_eq!(json["install_config"]["ssh_authorized_keys_count"], 2);
        assert!(json["install_config"]["password_set"].as_bool().unwrap());
        assert_eq!(json["install_config"]["timezone"], "America/New_York");
        assert_eq!(
            json["manifest_path"],
            "/run/live/medium/images/partitions.json"
        );
        assert!(json["copy_install_log"].as_bool().unwrap());
        assert!(json["config_warnings"].as_array().unwrap().is_empty());
    }

    // r[verify installer.dryrun.schema+5]
    #[test]
    fn plan_serializes_minimal() {
        let plan = InstallPlan::builder(&OperatingMode::Interactive, DiskEncryption::None)
            .config_warnings(vec!["some warning".into()])
            .build();

        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(json["mode"], "interactive");
        assert_eq!(json["disk_encryption"], "none");
        assert_eq!(json["variant"], "cloud");
        assert!(json["disk"].is_null());
        assert!(!json["tpm_present"].as_bool().unwrap());
        assert!(json["install_config"].is_null());
        assert!(json["manifest_path"].is_null());
        assert!(json["copy_install_log"].as_bool().unwrap());
        assert_eq!(json["config_warnings"][0], "some warning");
    }

    // r[verify installer.dryrun.schema+5]
    #[test]
    fn plan_keyfile_derives_metal_variant() {
        let plan = InstallPlan::builder(&OperatingMode::Auto, DiskEncryption::Keyfile).build();

        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(json["disk_encryption"], "keyfile");
        assert_eq!(json["variant"], "metal");
        assert!(!json["tpm_present"].as_bool().unwrap());
    }

    // r[verify installer.dryrun.schema+5]
    #[test]
    fn plan_copy_install_log_false() {
        let plan = InstallPlan::builder(&OperatingMode::Auto, DiskEncryption::None)
            .copy_install_log(false)
            .build();

        let json = serde_json::to_value(&plan).unwrap();
        assert!(!json["copy_install_log"].as_bool().unwrap());
    }

    // r[verify installer.dryrun.schema+5]
    #[test]
    fn install_config_info_hides_authkey_value() {
        let cfg = InstallConfig {
            tailscale_authkey: Some("tskey-auth-secret-value".into()),
            ..Default::default()
        };
        let info = InstallConfigInfo::from_config(&cfg, false, "UTC");
        assert!(info.tailscale_authkey);

        let json = serde_json::to_value(&info).unwrap();
        assert!(json["tailscale_authkey"].is_boolean());
        assert!(json["tailscale_authkey"].as_bool().unwrap());
    }

    // r[verify installer.dryrun.schema+5]
    #[test]
    fn install_config_info_no_authkey() {
        let cfg = InstallConfig {
            hostname: Some("host".into()),
            ..Default::default()
        };
        let info = InstallConfigInfo::from_config(&cfg, false, "UTC");
        assert!(!info.tailscale_authkey);
        assert!(!info.password_set);
    }

    // r[verify installer.dryrun.schema+5]
    #[test]
    fn install_config_info_password_set_from_plaintext() {
        let cfg = InstallConfig {
            password: Some("secret".into()),
            ..Default::default()
        };
        let info = InstallConfigInfo::from_config(&cfg, false, "UTC");
        assert!(info.password_set);
    }

    // r[verify installer.dryrun.schema+5]
    #[test]
    fn install_config_info_password_set_from_hash() {
        let cfg = InstallConfig {
            password_hash: Some("$6$rounds=4096$salt$hash".into()),
            ..Default::default()
        };
        let info = InstallConfigInfo::from_config(&cfg, false, "UTC");
        assert!(info.password_set);
    }

    // r[verify installer.dryrun.schema+5]
    #[test]
    fn disk_info_from_block_device() {
        let dev = sample_device();
        let info = DiskInfo::from(&dev);
        assert_eq!(info.path, PathBuf::from("/dev/nvme0n1"));
        assert_eq!(info.model, "Samsung 980 PRO");
        assert_eq!(info.size_bytes, 1_000_204_886_016);
        assert_eq!(info.transport, "NVMe");
    }

    // r[verify installer.dryrun.schema+5]
    #[test]
    fn all_operating_modes_map_correctly() {
        let dev = sample_device();

        let cases: Vec<(OperatingMode, &str)> = vec![
            (OperatingMode::Interactive, "interactive"),
            (OperatingMode::Prefilled, "prefilled"),
            (OperatingMode::Auto, "auto"),
            (
                OperatingMode::AutoIncomplete {
                    missing_disk_encryption: true,
                    missing_disk: true,
                    missing_hostname: false,
                },
                "auto-incomplete",
            ),
        ];

        for (mode, expected_str) in cases {
            let plan = InstallPlan::builder(&mode, DiskEncryption::Tpm)
                .disk(&dev)
                .tpm_present(true)
                .build();
            assert_eq!(plan.mode, expected_str);
        }
    }

    // r[verify installer.dryrun.output]
    #[test]
    fn write_to_path_creates_valid_json_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.json");

        let plan = InstallPlan::builder(&OperatingMode::Auto, DiskEncryption::Tpm)
            .tpm_present(true)
            .build();
        plan.write_to_path(&path).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed["mode"], "auto");
        assert_eq!(parsed["variant"], "metal");
        assert_eq!(parsed["disk_encryption"], "tpm");
        assert!(parsed["tpm_present"].as_bool().unwrap());
    }

    // r[verify installer.dryrun.schema+5]
    #[test]
    fn install_config_info_dhcp_hostname_sentinel() {
        let cfg = InstallConfig {
            hostname_from_dhcp: true,
            ..Default::default()
        };
        let info = InstallConfigInfo::from_config(&cfg, false, "UTC");
        assert_eq!(info.hostname.as_deref(), Some("dhcp"));
        assert!(!info.hostname_from_template);
    }

    // r[verify installer.dryrun.schema+5]
    #[test]
    fn install_config_info_template_flag() {
        let cfg = InstallConfig {
            hostname: Some("srv-a1b2c3".into()),
            ..Default::default()
        };
        let info = InstallConfigInfo::from_config(&cfg, true, "Pacific/Auckland");
        assert_eq!(info.hostname.as_deref(), Some("srv-a1b2c3"));
        assert!(info.hostname_from_template);
        assert_eq!(info.timezone, "Pacific/Auckland");
    }

    // r[verify installer.dryrun.schema+5]
    #[test]
    fn install_config_info_timezone_default() {
        let cfg = InstallConfig::default();
        let info = InstallConfigInfo::from_config(&cfg, false, "UTC");
        assert_eq!(info.timezone, "UTC");
    }
}
