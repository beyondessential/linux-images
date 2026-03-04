use std::path::PathBuf;

use serde::Serialize;

use crate::config::{FirstbootConfig, OperatingMode, Variant};
use crate::disk::BlockDevice;

// r[impl installer.dryrun.schema+2]
#[derive(Debug, Clone, Serialize)]
pub struct InstallPlan {
    pub mode: String,
    pub variant: String,
    pub disk: Option<DiskInfo>,
    pub disable_tpm: bool,
    pub firstboot: Option<FirstbootInfo>,
    pub image_path: Option<PathBuf>,
    pub config_warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskInfo {
    pub path: PathBuf,
    pub model: String,
    pub size_bytes: u64,
    pub transport: String,
}

// r[impl installer.dryrun.schema+2]
#[derive(Debug, Clone, Serialize)]
pub struct FirstbootInfo {
    pub hostname: Option<String>,
    pub tailscale_authkey: bool,
    pub ssh_authorized_keys_count: usize,
    pub password_set: bool,
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

impl From<&FirstbootConfig> for FirstbootInfo {
    fn from(fb: &FirstbootConfig) -> Self {
        FirstbootInfo {
            hostname: fb.hostname.clone(),
            tailscale_authkey: fb.tailscale_authkey.is_some(),
            ssh_authorized_keys_count: fb.ssh_authorized_keys.len(),
            password_set: fb.has_password(),
        }
    }
}

impl InstallPlan {
    pub fn new(
        mode: &OperatingMode,
        variant: Variant,
        disk: Option<&BlockDevice>,
        disable_tpm: bool,
        firstboot: Option<&FirstbootConfig>,
        image_path: Option<PathBuf>,
        config_warnings: Vec<String>,
    ) -> Self {
        let mode_str = match mode {
            OperatingMode::Interactive => "interactive",
            OperatingMode::Prefilled => "prefilled",
            OperatingMode::Auto => "auto",
            OperatingMode::AutoIncomplete { .. } => "auto-incomplete",
        };

        InstallPlan {
            mode: mode_str.to_string(),
            variant: variant.to_string(),
            disk: disk.map(DiskInfo::from),
            disable_tpm,
            firstboot: firstboot.map(FirstbootInfo::from),
            image_path,
            config_warnings,
        }
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
    use crate::config::{FirstbootConfig, OperatingMode, Variant};
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

    fn sample_firstboot() -> FirstbootConfig {
        FirstbootConfig {
            hostname: Some("server-01".into()),
            tailscale_authkey: Some("tskey-auth-xxxxx".into()),
            ssh_authorized_keys: vec!["ssh-ed25519 AAAA key1".into(), "ssh-rsa BBBB key2".into()],
            password: Some("changeme".into()),
            password_hash: None,
        }
    }

    // r[verify installer.dryrun.schema+2]
    #[test]
    fn plan_serializes_full() {
        let dev = sample_device();
        let fb = sample_firstboot();
        let plan = InstallPlan::new(
            &OperatingMode::Auto,
            Variant::Metal,
            Some(&dev),
            false,
            Some(&fb),
            Some(PathBuf::from("/run/live/medium/images/metal-amd64.raw.zst")),
            vec![],
        );

        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(json["mode"], "auto");
        assert_eq!(json["variant"], "metal");
        assert_eq!(json["disk"]["path"], "/dev/nvme0n1");
        assert_eq!(json["disk"]["model"], "Samsung 980 PRO");
        assert_eq!(json["disk"]["size_bytes"], 1_000_204_886_016u64);
        assert_eq!(json["disk"]["transport"], "NVMe");
        assert!(!json["disable_tpm"].as_bool().unwrap());
        assert_eq!(json["firstboot"]["hostname"], "server-01");
        assert!(json["firstboot"]["tailscale_authkey"].as_bool().unwrap());
        assert_eq!(json["firstboot"]["ssh_authorized_keys_count"], 2);
        assert!(json["firstboot"]["password_set"].as_bool().unwrap());
        assert_eq!(
            json["image_path"],
            "/run/live/medium/images/metal-amd64.raw.zst"
        );
        assert!(json["config_warnings"].as_array().unwrap().is_empty());
    }

    // r[verify installer.dryrun.schema+2]
    #[test]
    fn plan_serializes_minimal() {
        let plan = InstallPlan::new(
            &OperatingMode::Interactive,
            Variant::Cloud,
            None,
            false,
            None,
            None,
            vec!["some warning".into()],
        );

        let json = serde_json::to_value(&plan).unwrap();
        assert_eq!(json["mode"], "interactive");
        assert_eq!(json["variant"], "cloud");
        assert!(json["disk"].is_null());
        assert!(json["firstboot"].is_null());
        assert!(json["image_path"].is_null());
        assert_eq!(json["config_warnings"][0], "some warning");
    }

    // r[verify installer.dryrun.schema+2]
    #[test]
    fn firstboot_info_hides_authkey_value() {
        let fb = FirstbootConfig {
            hostname: None,
            tailscale_authkey: Some("tskey-auth-secret-value".into()),
            ssh_authorized_keys: vec![],
            password: None,
            password_hash: None,
        };
        let info = FirstbootInfo::from(&fb);
        assert!(info.tailscale_authkey);

        let json = serde_json::to_value(&info).unwrap();
        assert!(json["tailscale_authkey"].is_boolean());
        assert!(json["tailscale_authkey"].as_bool().unwrap());
    }

    // r[verify installer.dryrun.schema+2]
    #[test]
    fn firstboot_info_no_authkey() {
        let fb = FirstbootConfig {
            hostname: Some("host".into()),
            tailscale_authkey: None,
            ssh_authorized_keys: vec![],
            password: None,
            password_hash: None,
        };
        let info = FirstbootInfo::from(&fb);
        assert!(!info.tailscale_authkey);
        assert!(!info.password_set);
    }

    // r[verify installer.dryrun.schema+2]
    #[test]
    fn firstboot_info_password_set_from_plaintext() {
        let fb = FirstbootConfig {
            hostname: None,
            tailscale_authkey: None,
            ssh_authorized_keys: vec![],
            password: Some("secret".into()),
            password_hash: None,
        };
        let info = FirstbootInfo::from(&fb);
        assert!(info.password_set);
    }

    // r[verify installer.dryrun.schema+2]
    #[test]
    fn firstboot_info_password_set_from_hash() {
        let fb = FirstbootConfig {
            hostname: None,
            tailscale_authkey: None,
            ssh_authorized_keys: vec![],
            password: None,
            password_hash: Some("$6$rounds=4096$salt$hash".into()),
        };
        let info = FirstbootInfo::from(&fb);
        assert!(info.password_set);
    }

    // r[verify installer.dryrun.schema+2]
    #[test]
    fn disk_info_from_block_device() {
        let dev = sample_device();
        let info = DiskInfo::from(&dev);
        assert_eq!(info.path, PathBuf::from("/dev/nvme0n1"));
        assert_eq!(info.model, "Samsung 980 PRO");
        assert_eq!(info.size_bytes, 1_000_204_886_016);
        assert_eq!(info.transport, "NVMe");
    }

    // r[verify installer.dryrun.schema+2]
    #[test]
    fn all_operating_modes_map_correctly() {
        let dev = sample_device();

        let cases: Vec<(OperatingMode, &str)> = vec![
            (OperatingMode::Interactive, "interactive"),
            (OperatingMode::Prefilled, "prefilled"),
            (OperatingMode::Auto, "auto"),
            (
                OperatingMode::AutoIncomplete {
                    missing_variant: true,
                    missing_disk: true,
                    missing_hostname: false,
                },
                "auto-incomplete",
            ),
        ];

        for (mode, expected_str) in cases {
            let plan =
                InstallPlan::new(&mode, Variant::Metal, Some(&dev), false, None, None, vec![]);
            assert_eq!(plan.mode, expected_str);
        }
    }

    // r[verify installer.dryrun.output]
    #[test]
    fn write_to_path_creates_valid_json_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.json");

        let plan = InstallPlan::new(
            &OperatingMode::Auto,
            Variant::Metal,
            None,
            true,
            None,
            None,
            vec![],
        );
        plan.write_to_path(&path).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed["mode"], "auto");
        assert_eq!(parsed["variant"], "metal");
        assert!(parsed["disable_tpm"].as_bool().unwrap());
    }
}
