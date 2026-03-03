use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

// r[impl installer.config.schema]
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InstallConfig {
    #[serde(default)]
    pub auto: bool,

    pub variant: Option<Variant>,

    pub disk: Option<DiskSelector>,

    #[serde(default, rename = "disable-tpm")]
    pub disable_tpm: bool,

    pub firstboot: Option<FirstbootConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Variant {
    Metal,
    Cloud,
}

impl fmt::Display for Variant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Variant::Metal => write!(f, "metal"),
            Variant::Cloud => write!(f, "cloud"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskSelector {
    Path(PathBuf),
    Strategy(DiskStrategy),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskStrategy {
    LargestSsd,
    Largest,
    Smallest,
}

impl fmt::Display for DiskStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiskStrategy::LargestSsd => write!(f, "largest-ssd"),
            DiskStrategy::Largest => write!(f, "largest"),
            DiskStrategy::Smallest => write!(f, "smallest"),
        }
    }
}

impl fmt::Display for DiskSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiskSelector::Path(p) => write!(f, "{}", p.display()),
            DiskSelector::Strategy(s) => write!(f, "{s}"),
        }
    }
}

impl DiskSelector {
    pub fn parse(s: &str) -> Self {
        match s {
            "largest-ssd" => DiskSelector::Strategy(DiskStrategy::LargestSsd),
            "largest" => DiskSelector::Strategy(DiskStrategy::Largest),
            "smallest" => DiskSelector::Strategy(DiskStrategy::Smallest),
            other => DiskSelector::Path(PathBuf::from(other)),
        }
    }
}

impl<'de> Deserialize<'de> for DiskSelector {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(DiskSelector::parse(&s))
    }
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FirstbootConfig {
    pub hostname: Option<String>,

    #[serde(default, rename = "tailscale-authkey")]
    pub tailscale_authkey: Option<String>,

    #[serde(default, rename = "ssh-authorized-keys")]
    pub ssh_authorized_keys: Vec<String>,
}

// r[impl installer.mode.interactive]
// r[impl installer.mode.prefilled]
// r[impl installer.mode.auto]
// r[impl installer.mode.auto-incomplete]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatingMode {
    Interactive,
    Prefilled,
    Auto,
    AutoIncomplete {
        missing_variant: bool,
        missing_disk: bool,
    },
}

impl fmt::Display for OperatingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OperatingMode::Interactive => write!(f, "interactive"),
            OperatingMode::Prefilled => write!(f, "prefilled"),
            OperatingMode::Auto => write!(f, "automatic"),
            OperatingMode::AutoIncomplete { .. } => write!(f, "automatic (incomplete config)"),
        }
    }
}

impl InstallConfig {
    pub fn load_from_file(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            tracing::info!("no config file at {}", path.display());
            return Ok(None);
        }
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("reading config file: {}", path.display()))?;
        let config: InstallConfig = toml::from_str(&contents)
            .with_context(|| format!("parsing config file: {}", path.display()))?;
        tracing::info!("loaded config from {}", path.display());
        Ok(Some(config))
    }

    #[cfg(test)]
    pub fn from_toml(s: &str) -> Result<Self> {
        toml::from_str(s).context("parsing TOML config")
    }

    pub fn mode(&self) -> OperatingMode {
        if !self.auto {
            return OperatingMode::Prefilled;
        }
        if self.variant.is_some() && self.disk.is_some() {
            OperatingMode::Auto
        } else {
            OperatingMode::AutoIncomplete {
                missing_variant: self.variant.is_none(),
                missing_disk: self.disk.is_none(),
            }
        }
    }

    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();

        if self.disable_tpm && self.variant == Some(Variant::Cloud) {
            issues.push("disable-tpm has no effect with the cloud variant".into());
        }

        if let Some(ref fb) = self.firstboot {
            if let Some(ref hostname) = fb.hostname {
                if hostname.is_empty() {
                    issues.push("firstboot.hostname is empty".into());
                }
                if hostname.len() > 63 {
                    issues.push(format!(
                        "firstboot.hostname is too long ({} chars, max 63)",
                        hostname.len()
                    ));
                }
                let valid = !hostname.starts_with('-')
                    && !hostname.ends_with('-')
                    && hostname
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-');
                if !valid {
                    issues.push(format!(
                        "firstboot.hostname '{}' is not a valid hostname",
                        hostname
                    ));
                }
            }

            for (i, key) in fb.ssh_authorized_keys.iter().enumerate() {
                if key.trim().is_empty() {
                    issues.push(format!("firstboot.ssh-authorized-keys[{i}] is empty"));
                }
            }
        }

        issues
    }
}

pub fn find_config_file() -> Option<PathBuf> {
    // r[impl installer.config.location]
    let candidates = [
        // BESCONF partition (appended FAT32 partition on USB-booted ISO)
        PathBuf::from("/run/besconf/bes-install.toml"),
        // ISO filesystem root (mounted by live-boot)
        PathBuf::from("/run/live/medium/bes-install.toml"),
        // Legacy / manual placement paths
        PathBuf::from("/boot/efi/bes-install.toml"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            tracing::info!("found config file at {}", candidate.display());
            return Some(candidate.clone());
        }
    }

    tracing::info!("no config file found at any known location");
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify installer.config.schema]
    #[test]
    fn parse_empty_config() {
        let config = InstallConfig::from_toml("").unwrap();
        assert_eq!(config, InstallConfig::default());
    }

    // r[verify installer.config.schema]
    #[test]
    fn parse_full_config() {
        let toml = r#"
            auto = true
            variant = "metal"
            disk = "largest-ssd"
            disable-tpm = false

            [firstboot]
            hostname = "server-01"
            tailscale-authkey = "tskey-auth-xxxxx"
            ssh-authorized-keys = ["ssh-ed25519 AAAA... admin@example.com"]
        "#;
        let config = InstallConfig::from_toml(toml).unwrap();
        assert!(config.auto);
        assert_eq!(config.variant, Some(Variant::Metal));
        assert_eq!(
            config.disk,
            Some(DiskSelector::Strategy(DiskStrategy::LargestSsd))
        );
        assert!(!config.disable_tpm);

        let fb = config.firstboot.as_ref().unwrap();
        assert_eq!(fb.hostname.as_deref(), Some("server-01"));
        assert_eq!(fb.tailscale_authkey.as_deref(), Some("tskey-auth-xxxxx"));
        assert_eq!(
            fb.ssh_authorized_keys,
            vec!["ssh-ed25519 AAAA... admin@example.com"]
        );
    }

    // r[verify installer.config.schema]
    #[test]
    fn parse_cloud_variant() {
        let config = InstallConfig::from_toml(r#"variant = "cloud""#).unwrap();
        assert_eq!(config.variant, Some(Variant::Cloud));
    }

    // r[verify installer.config.schema]
    #[test]
    fn parse_disk_path() {
        let config = InstallConfig::from_toml(r#"disk = "/dev/nvme0n1""#).unwrap();
        assert_eq!(
            config.disk,
            Some(DiskSelector::Path(PathBuf::from("/dev/nvme0n1")))
        );
    }

    // r[verify installer.config.schema]
    #[test]
    fn parse_disk_strategies() {
        for (input, expected) in [
            ("largest-ssd", DiskStrategy::LargestSsd),
            ("largest", DiskStrategy::Largest),
            ("smallest", DiskStrategy::Smallest),
        ] {
            let config = InstallConfig::from_toml(&format!(r#"disk = "{input}""#)).unwrap();
            assert_eq!(config.disk, Some(DiskSelector::Strategy(expected)));
        }
    }

    // r[verify installer.config.schema]
    #[test]
    fn parse_invalid_variant_rejected() {
        let result = InstallConfig::from_toml(r#"variant = "bad""#);
        assert!(result.is_err());
    }

    // r[verify installer.config.schema]
    #[test]
    fn parse_unknown_field_rejected() {
        let result = InstallConfig::from_toml(r#"bogus = true"#);
        assert!(result.is_err());
    }

    // r[verify installer.mode.prefilled]
    #[test]
    fn mode_prefilled_when_auto_false() {
        let config = InstallConfig::default();
        assert_eq!(config.mode(), OperatingMode::Prefilled);
    }

    // r[verify installer.mode.auto]
    #[test]
    fn mode_auto_when_complete() {
        let config = InstallConfig {
            auto: true,
            variant: Some(Variant::Metal),
            disk: Some(DiskSelector::Strategy(DiskStrategy::LargestSsd)),
            ..Default::default()
        };
        assert_eq!(config.mode(), OperatingMode::Auto);
    }

    // r[verify installer.mode.auto-incomplete]
    #[test]
    fn mode_auto_incomplete_missing_variant() {
        let config = InstallConfig {
            auto: true,
            disk: Some(DiskSelector::Strategy(DiskStrategy::Largest)),
            ..Default::default()
        };
        assert_eq!(
            config.mode(),
            OperatingMode::AutoIncomplete {
                missing_variant: true,
                missing_disk: false,
            }
        );
    }

    // r[verify installer.mode.auto-incomplete]
    #[test]
    fn mode_auto_incomplete_missing_disk() {
        let config = InstallConfig {
            auto: true,
            variant: Some(Variant::Cloud),
            ..Default::default()
        };
        assert_eq!(
            config.mode(),
            OperatingMode::AutoIncomplete {
                missing_variant: false,
                missing_disk: true,
            }
        );
    }

    // r[verify installer.mode.auto-incomplete]
    #[test]
    fn mode_auto_incomplete_missing_both() {
        let config = InstallConfig {
            auto: true,
            ..Default::default()
        };
        assert_eq!(
            config.mode(),
            OperatingMode::AutoIncomplete {
                missing_variant: true,
                missing_disk: true,
            }
        );
    }

    // r[verify installer.config.schema]
    #[test]
    fn validate_disable_tpm_cloud_warns() {
        let config = InstallConfig {
            variant: Some(Variant::Cloud),
            disable_tpm: true,
            ..Default::default()
        };
        let issues = config.validate();
        assert!(
            issues
                .iter()
                .any(|i| i.contains("disable-tpm has no effect"))
        );
    }

    // r[verify installer.config.schema]
    #[test]
    fn validate_bad_hostname() {
        let config = InstallConfig {
            firstboot: Some(FirstbootConfig {
                hostname: Some("-bad-".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let issues = config.validate();
        assert!(issues.iter().any(|i| i.contains("not a valid hostname")));
    }

    // r[verify installer.config.schema]
    #[test]
    fn validate_long_hostname() {
        let config = InstallConfig {
            firstboot: Some(FirstbootConfig {
                hostname: Some("a".repeat(64)),
                ..Default::default()
            }),
            ..Default::default()
        };
        let issues = config.validate();
        assert!(issues.iter().any(|i| i.contains("too long")));
    }

    // r[verify installer.config.schema]
    #[test]
    fn validate_empty_ssh_key() {
        let config = InstallConfig {
            firstboot: Some(FirstbootConfig {
                ssh_authorized_keys: vec!["".into()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let issues = config.validate();
        assert!(issues.iter().any(|i| i.contains("empty")));
    }

    // r[verify installer.config.schema]
    #[test]
    fn validate_good_config_has_no_issues() {
        let config = InstallConfig {
            auto: true,
            variant: Some(Variant::Metal),
            disk: Some(DiskSelector::Strategy(DiskStrategy::LargestSsd)),
            disable_tpm: false,
            firstboot: Some(FirstbootConfig {
                hostname: Some("server-01".into()),
                tailscale_authkey: Some("tskey-auth-xxxxx".into()),
                ssh_authorized_keys: vec!["ssh-ed25519 AAAA... admin@example.com".into()],
            }),
        };
        let issues = config.validate();
        assert!(issues.is_empty(), "unexpected issues: {issues:?}");
    }

    // r[verify installer.config.schema]
    #[test]
    fn variant_display() {
        assert_eq!(Variant::Metal.to_string(), "metal");
        assert_eq!(Variant::Cloud.to_string(), "cloud");
    }

    // r[verify installer.config.schema]
    #[test]
    fn disk_selector_display() {
        assert_eq!(
            DiskSelector::Strategy(DiskStrategy::LargestSsd).to_string(),
            "largest-ssd"
        );
        assert_eq!(
            DiskSelector::Path(PathBuf::from("/dev/sda")).to_string(),
            "/dev/sda"
        );
    }

    // r[verify installer.config.schema]
    #[test]
    fn parse_minimal_firstboot() {
        let config = InstallConfig::from_toml(
            r#"
            [firstboot]
            hostname = "test"
        "#,
        )
        .unwrap();
        let fb = config.firstboot.unwrap();
        assert_eq!(fb.hostname.as_deref(), Some("test"));
        assert_eq!(fb.tailscale_authkey, None);
        assert!(fb.ssh_authorized_keys.is_empty());
    }

    // r[verify installer.config.location]
    #[test]
    fn load_nonexistent_file_returns_none() {
        let result = InstallConfig::load_from_file(Path::new("/nonexistent/bes-install.toml"));
        assert!(result.unwrap().is_none());
    }

    // r[verify installer.config.location]
    #[test]
    fn load_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bes-install.toml");
        std::fs::write(
            &path,
            r#"
            variant = "cloud"
            disk = "smallest"
        "#,
        )
        .unwrap();
        let config = InstallConfig::load_from_file(&path).unwrap().unwrap();
        assert_eq!(config.variant, Some(Variant::Cloud));
        assert_eq!(
            config.disk,
            Some(DiskSelector::Strategy(DiskStrategy::Smallest))
        );
    }

    // r[verify installer.config.location]
    #[test]
    fn load_invalid_toml_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bes-install.toml");
        std::fs::write(&path, "this is not valid toml {{{{").unwrap();
        assert!(InstallConfig::load_from_file(&path).is_err());
    }

    // r[verify installer.mode.interactive]
    // r[verify installer.mode.prefilled]
    // r[verify installer.mode.auto]
    // r[verify installer.mode.auto-incomplete]
    #[test]
    fn operating_mode_display() {
        assert_eq!(OperatingMode::Interactive.to_string(), "interactive");
        assert_eq!(OperatingMode::Prefilled.to_string(), "prefilled");
        assert_eq!(OperatingMode::Auto.to_string(), "automatic");
        assert_eq!(
            OperatingMode::AutoIncomplete {
                missing_variant: true,
                missing_disk: true,
            }
            .to_string(),
            "automatic (incomplete config)"
        );
    }
}
