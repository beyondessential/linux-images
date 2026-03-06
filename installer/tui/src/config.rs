use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::hostname_template;

// r[impl installer.config.format]
// r[impl installer.config.auto]
// r[impl installer.config.disk-encryption]
// r[impl installer.config.disk]
// r[impl installer.config.copy-install-log]
// r[impl installer.config.hostname]
// r[impl installer.config.tailscale-authkey+3]
// r[impl installer.config.ssh-authorized-keys+2]
// r[impl installer.config.password]
// r[impl installer.config.timezone]
#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct InstallConfig {
    #[serde(default)]
    pub auto: bool,

    #[serde(default, rename = "disk-encryption")]
    pub disk_encryption: Option<DiskEncryption>,

    pub disk: Option<DiskSelector>,

    #[serde(default, rename = "copy-install-log")]
    pub copy_install_log: Option<bool>,

    pub hostname: Option<String>,

    #[serde(default, rename = "hostname-from-dhcp")]
    pub hostname_from_dhcp: bool,

    #[serde(default, rename = "hostname-template")]
    pub hostname_template: Option<String>,

    #[serde(default, rename = "tailscale-authkey")]
    pub tailscale_authkey: Option<String>,

    #[serde(default, rename = "ssh-authorized-keys")]
    pub ssh_authorized_keys: Vec<String>,

    pub password: Option<String>,

    #[serde(default, rename = "password-hash")]
    pub password_hash: Option<String>,

    pub timezone: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskEncryption {
    Tpm,
    Keyfile,
    None,
}

impl DiskEncryption {
    pub fn variant(self) -> Variant {
        match self {
            DiskEncryption::Tpm | DiskEncryption::Keyfile => Variant::Metal,
            DiskEncryption::None => Variant::Cloud,
        }
    }

    pub fn is_encrypted(self) -> bool {
        matches!(self, DiskEncryption::Tpm | DiskEncryption::Keyfile)
    }
}

impl fmt::Display for DiskEncryption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiskEncryption::Tpm => write!(f, "tpm"),
            DiskEncryption::Keyfile => write!(f, "keyfile"),
            DiskEncryption::None => write!(f, "none"),
        }
    }
}

impl<'de> Deserialize<'de> for DiskEncryption {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "tpm" => Ok(DiskEncryption::Tpm),
            "keyfile" => Ok(DiskEncryption::Keyfile),
            "none" => Ok(DiskEncryption::None),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["tpm", "keyfile", "none"],
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl InstallConfig {
    pub fn has_password(&self) -> bool {
        self.password.is_some() || self.password_hash.is_some()
    }

    pub fn has_hostname_config(&self) -> bool {
        self.hostname.is_some() || self.hostname_from_dhcp || self.hostname_template.is_some()
    }

    pub fn has_install_config_fields(&self) -> bool {
        self.has_hostname_config()
            || self.tailscale_authkey.is_some()
            || !self.ssh_authorized_keys.is_empty()
            || self.has_password()
            || self.timezone.is_some()
    }
}

/// Validate a hostname per RFC 1123: ASCII alphanumeric and hyphens only,
/// must not start or end with a hyphen, max 63 characters.
/// Returns `Ok(())` if valid, or `Err` with a human-readable description.
pub fn validate_hostname(hostname: &str) -> Result<(), String> {
    if hostname.is_empty() {
        return Err("Hostname cannot be empty.".into());
    }
    if hostname.len() > 63 {
        return Err(format!(
            "Hostname is too long ({} chars, max 63).",
            hostname.len()
        ));
    }
    if hostname.starts_with('-') || hostname.ends_with('-') {
        return Err("Hostname must not start or end with a hyphen.".into());
    }
    if !hostname
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        return Err("Hostname must contain only letters, digits, and hyphens.".into());
    }
    Ok(())
}

// r[impl installer.mode.interactive+2]
// r[impl installer.mode.prefilled]
// r[impl installer.mode.auto+4]
// r[impl installer.mode.auto-incomplete+3]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatingMode {
    Interactive,
    Prefilled,
    Auto,
    AutoIncomplete {
        missing_disk_encryption: bool,
        missing_disk: bool,
        missing_hostname: bool,
    },
}

impl fmt::Display for OperatingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OperatingMode::Interactive => write!(f, "interactive"),
            OperatingMode::Prefilled => write!(f, "prefilled"),
            OperatingMode::Auto => write!(f, "automatic"),
            OperatingMode::AutoIncomplete { .. } => {
                write!(f, "automatic (incomplete config)")
            }
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

    // r[impl installer.mode.auto+4]
    // r[impl installer.mode.auto-incomplete+3]
    pub fn mode(&self) -> OperatingMode {
        if !self.auto {
            return OperatingMode::Prefilled;
        }

        let missing_disk_encryption = self.disk_encryption.is_none();
        let missing_disk = self.disk.is_none();
        let missing_hostname =
            self.disk_encryption.is_some_and(|de| de.is_encrypted()) && !self.has_hostname_config();

        if !missing_disk_encryption && !missing_disk && !missing_hostname {
            OperatingMode::Auto
        } else {
            OperatingMode::AutoIncomplete {
                missing_disk_encryption,
                missing_disk,
                missing_hostname,
            }
        }
    }

    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();

        // Three-way mutual exclusivity for hostname fields
        let hostname_set = self.hostname.is_some();
        let dhcp_set = self.hostname_from_dhcp;
        let template_set = self.hostname_template.is_some();
        let hostname_count = hostname_set as u8 + dhcp_set as u8 + template_set as u8;
        if hostname_count > 1 {
            let mut conflicting = Vec::new();
            if hostname_set {
                conflicting.push("hostname");
            }
            if dhcp_set {
                conflicting.push("hostname-from-dhcp");
            }
            if template_set {
                conflicting.push("hostname-template");
            }
            issues.push(format!(
                "{} are mutually exclusive",
                conflicting.join(" and ")
            ));
        }

        if let Some(ref hostname) = self.hostname
            && let Err(e) = validate_hostname(hostname)
        {
            issues.push(format!("hostname '{}': {}", hostname, e));
        }

        if let Some(ref tmpl) = self.hostname_template
            && let Err(e) = hostname_template::parse(tmpl)
        {
            issues.push(format!("hostname-template: {e}"));
        }

        if self.hostname_from_dhcp
            && self
                .disk_encryption
                .is_some_and(|de| de == DiskEncryption::None)
        {
            issues.push(
                "hostname-from-dhcp has no special effect with disk-encryption = \"none\" (DHCP hostname is already the default)".into(),
            );
        }

        for (i, key) in self.ssh_authorized_keys.iter().enumerate() {
            if key.trim().is_empty() {
                issues.push(format!("ssh-authorized-keys[{i}] is empty"));
            }
        }

        if self.password.is_some() && self.password_hash.is_some() {
            issues.push("password and password-hash are mutually exclusive".into());
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

    // r[verify installer.config.format]
    #[test]
    fn parse_empty_config() {
        let config = InstallConfig::from_toml("").unwrap();
        assert_eq!(config, InstallConfig::default());
    }

    // r[verify installer.config.format]
    // r[verify installer.config.auto]
    // r[verify installer.config.disk-encryption]
    // r[verify installer.config.disk]
    // r[verify installer.config.hostname]
    // r[verify installer.config.tailscale-authkey+3]
    // r[verify installer.config.ssh-authorized-keys+2]
    // r[verify installer.config.password]
    #[test]
    fn parse_full_config() {
        let toml = r#"
            auto = true
            disk-encryption = "tpm"
            disk = "largest-ssd"
            hostname = "server-01"
            tailscale-authkey = "tskey-auth-xxxxx"
            ssh-authorized-keys = ["ssh-ed25519 AAAA... admin@example.com"]
            password = "changeme"
        "#;
        let config = InstallConfig::from_toml(toml).unwrap();
        assert!(config.auto);
        assert_eq!(config.disk_encryption, Some(DiskEncryption::Tpm));
        assert_eq!(
            config.disk,
            Some(DiskSelector::Strategy(DiskStrategy::LargestSsd))
        );

        assert_eq!(config.hostname.as_deref(), Some("server-01"));
        assert_eq!(
            config.tailscale_authkey.as_deref(),
            Some("tskey-auth-xxxxx")
        );
        assert_eq!(
            config.ssh_authorized_keys,
            vec!["ssh-ed25519 AAAA... admin@example.com"]
        );
        assert_eq!(config.password.as_deref(), Some("changeme"));
        assert_eq!(config.password_hash, None);
    }

    // r[verify installer.config.disk-encryption]
    #[test]
    fn parse_disk_encryption_variants() {
        let tpm = InstallConfig::from_toml(r#"disk-encryption = "tpm""#).unwrap();
        assert_eq!(tpm.disk_encryption, Some(DiskEncryption::Tpm));

        let keyfile = InstallConfig::from_toml(r#"disk-encryption = "keyfile""#).unwrap();
        assert_eq!(keyfile.disk_encryption, Some(DiskEncryption::Keyfile));

        let none = InstallConfig::from_toml(r#"disk-encryption = "none""#).unwrap();
        assert_eq!(none.disk_encryption, Some(DiskEncryption::None));
    }

    // r[verify installer.config.disk-encryption]
    #[test]
    fn parse_invalid_disk_encryption_rejected() {
        let result = InstallConfig::from_toml(r#"disk-encryption = "bad""#);
        assert!(result.is_err());
    }

    // r[verify installer.config.disk]
    #[test]
    fn parse_disk_path() {
        let config = InstallConfig::from_toml(r#"disk = "/dev/nvme0n1""#).unwrap();
        assert_eq!(
            config.disk,
            Some(DiskSelector::Path(PathBuf::from("/dev/nvme0n1")))
        );
    }

    // r[verify installer.config.disk]
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

    // r[verify installer.config.format]
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

    // r[verify installer.mode.auto+4]
    #[test]
    fn mode_auto_when_complete_none() {
        let config = InstallConfig {
            auto: true,
            disk_encryption: Some(DiskEncryption::None),
            disk: Some(DiskSelector::Strategy(DiskStrategy::LargestSsd)),
            ..Default::default()
        };
        assert_eq!(config.mode(), OperatingMode::Auto);
    }

    // r[verify installer.mode.auto+4]
    #[test]
    fn mode_auto_when_complete_tpm_with_hostname() {
        let config = InstallConfig {
            auto: true,
            disk_encryption: Some(DiskEncryption::Tpm),
            disk: Some(DiskSelector::Strategy(DiskStrategy::LargestSsd)),
            hostname: Some("my-server".into()),
            ..Default::default()
        };
        assert_eq!(config.mode(), OperatingMode::Auto);
    }

    // r[verify installer.mode.auto+4]
    #[test]
    fn mode_auto_when_complete_keyfile_with_dhcp() {
        let config = InstallConfig {
            auto: true,
            disk_encryption: Some(DiskEncryption::Keyfile),
            disk: Some(DiskSelector::Strategy(DiskStrategy::LargestSsd)),
            hostname_from_dhcp: true,
            ..Default::default()
        };
        assert_eq!(config.mode(), OperatingMode::Auto);
    }

    // r[verify installer.mode.auto+4]
    #[test]
    fn mode_auto_when_complete_tpm_with_template() {
        let config = InstallConfig {
            auto: true,
            disk_encryption: Some(DiskEncryption::Tpm),
            disk: Some(DiskSelector::Strategy(DiskStrategy::LargestSsd)),
            hostname_template: Some("srv-{hex:6}".into()),
            ..Default::default()
        };
        assert_eq!(config.mode(), OperatingMode::Auto);
    }

    // r[verify installer.mode.auto-incomplete+3]
    #[test]
    fn mode_auto_incomplete_tpm_missing_hostname() {
        let config = InstallConfig {
            auto: true,
            disk_encryption: Some(DiskEncryption::Tpm),
            disk: Some(DiskSelector::Strategy(DiskStrategy::LargestSsd)),
            ..Default::default()
        };
        assert_eq!(
            config.mode(),
            OperatingMode::AutoIncomplete {
                missing_disk_encryption: false,
                missing_disk: false,
                missing_hostname: true,
            }
        );
    }

    // r[verify installer.mode.auto-incomplete+3]
    #[test]
    fn mode_auto_incomplete_keyfile_missing_hostname() {
        let config = InstallConfig {
            auto: true,
            disk_encryption: Some(DiskEncryption::Keyfile),
            disk: Some(DiskSelector::Strategy(DiskStrategy::LargestSsd)),
            ..Default::default()
        };
        assert_eq!(
            config.mode(),
            OperatingMode::AutoIncomplete {
                missing_disk_encryption: false,
                missing_disk: false,
                missing_hostname: true,
            }
        );
    }

    // r[verify installer.mode.auto-incomplete+3]
    #[test]
    fn mode_auto_incomplete_missing_disk_encryption() {
        let config = InstallConfig {
            auto: true,
            disk: Some(DiskSelector::Strategy(DiskStrategy::Largest)),
            ..Default::default()
        };
        assert_eq!(
            config.mode(),
            OperatingMode::AutoIncomplete {
                missing_disk_encryption: true,
                missing_disk: false,
                missing_hostname: false,
            }
        );
    }

    // r[verify installer.mode.auto-incomplete+3]
    #[test]
    fn mode_auto_incomplete_missing_disk() {
        let config = InstallConfig {
            auto: true,
            disk_encryption: Some(DiskEncryption::None),
            ..Default::default()
        };
        assert_eq!(
            config.mode(),
            OperatingMode::AutoIncomplete {
                missing_disk_encryption: false,
                missing_disk: true,
                missing_hostname: false,
            }
        );
    }

    // r[verify installer.mode.auto-incomplete+3]
    #[test]
    fn mode_auto_incomplete_missing_both() {
        let config = InstallConfig {
            auto: true,
            ..Default::default()
        };
        assert_eq!(
            config.mode(),
            OperatingMode::AutoIncomplete {
                missing_disk_encryption: true,
                missing_disk: true,
                missing_hostname: false,
            }
        );
    }

    // r[verify installer.mode.auto+4]
    #[test]
    fn mode_auto_none_does_not_require_hostname() {
        let config = InstallConfig {
            auto: true,
            disk_encryption: Some(DiskEncryption::None),
            disk: Some(DiskSelector::Strategy(DiskStrategy::LargestSsd)),
            ..Default::default()
        };
        assert_eq!(config.mode(), OperatingMode::Auto);
    }

    // r[verify installer.config.hostname]
    #[test]
    fn validate_bad_hostname() {
        let config = InstallConfig {
            hostname: Some("-bad-".into()),
            ..Default::default()
        };
        let issues = config.validate();
        assert!(
            issues
                .iter()
                .any(|i| i.contains("must not start or end with a hyphen"))
        );
    }

    // r[verify installer.config.hostname]
    #[test]
    fn validate_long_hostname() {
        let config = InstallConfig {
            hostname: Some("a".repeat(64)),
            ..Default::default()
        };
        let issues = config.validate();
        assert!(issues.iter().any(|i| i.contains("too long")));
    }

    // r[verify installer.config.ssh-authorized-keys+2]
    #[test]
    fn validate_empty_ssh_key() {
        let config = InstallConfig {
            ssh_authorized_keys: vec!["".into()],
            ..Default::default()
        };
        let issues = config.validate();
        assert!(issues.iter().any(|i| i.contains("empty")));
    }

    // r[verify installer.config.format]
    #[test]
    fn validate_good_config_has_no_issues() {
        let config = InstallConfig {
            auto: true,
            disk_encryption: Some(DiskEncryption::Tpm),
            disk: Some(DiskSelector::Strategy(DiskStrategy::LargestSsd)),
            hostname: Some("server-01".into()),
            tailscale_authkey: Some("tskey-auth-xxxxx".into()),
            ssh_authorized_keys: vec!["ssh-ed25519 AAAA... admin@example.com".into()],
            password: Some("changeme".into()),
            ..Default::default()
        };
        let issues = config.validate();
        assert!(issues.is_empty(), "unexpected issues: {issues:?}");
    }

    // r[verify installer.config.disk-encryption]
    #[test]
    fn disk_encryption_display() {
        assert_eq!(DiskEncryption::Tpm.to_string(), "tpm");
        assert_eq!(DiskEncryption::Keyfile.to_string(), "keyfile");
        assert_eq!(DiskEncryption::None.to_string(), "none");
    }

    // r[verify installer.config.disk-encryption]
    #[test]
    fn disk_encryption_variant_derivation() {
        assert_eq!(DiskEncryption::Tpm.variant(), Variant::Metal);
        assert_eq!(DiskEncryption::Keyfile.variant(), Variant::Metal);
        assert_eq!(DiskEncryption::None.variant(), Variant::Cloud);
    }

    // r[verify installer.config.disk-encryption]
    #[test]
    fn variant_display() {
        assert_eq!(Variant::Metal.to_string(), "metal");
        assert_eq!(Variant::Cloud.to_string(), "cloud");
    }

    // r[verify installer.config.disk]
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

    // r[verify installer.config.hostname]
    // r[verify installer.config.tailscale-authkey+3]
    // r[verify installer.config.ssh-authorized-keys+2]
    #[test]
    fn parse_minimal_hostname() {
        let config = InstallConfig::from_toml(
            r#"
            hostname = "test"
        "#,
        )
        .unwrap();
        assert_eq!(config.hostname.as_deref(), Some("test"));
        assert_eq!(config.tailscale_authkey, None);
        assert!(config.ssh_authorized_keys.is_empty());
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
            disk-encryption = "none"
            disk = "smallest"
        "#,
        )
        .unwrap();
        let config = InstallConfig::load_from_file(&path).unwrap().unwrap();
        assert_eq!(config.disk_encryption, Some(DiskEncryption::None));
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

    // r[verify installer.config.password]
    #[test]
    fn parse_password_hash() {
        let config = InstallConfig::from_toml(
            r#"
            password-hash = "$6$rounds=4096$salt$hash"
        "#,
        )
        .unwrap();
        assert_eq!(config.password, None);
        assert_eq!(
            config.password_hash.as_deref(),
            Some("$6$rounds=4096$salt$hash")
        );
    }

    // r[verify installer.config.password]
    #[test]
    fn validate_password_and_hash_mutually_exclusive() {
        let config = InstallConfig {
            password: Some("changeme".into()),
            password_hash: Some("$6$rounds=4096$salt$hash".into()),
            ..Default::default()
        };
        let issues = config.validate();
        assert!(
            issues
                .iter()
                .any(|i| i.contains("password") && i.contains("mutually exclusive"))
        );
    }

    #[test]
    fn validate_hostname_fields_mutually_exclusive() {
        let config = InstallConfig {
            hostname: Some("server-01".into()),
            hostname_from_dhcp: true,
            ..Default::default()
        };
        let issues = config.validate();
        assert!(issues.iter().any(|i| i.contains("mutually exclusive")));
    }

    #[test]
    fn validate_hostname_and_template_mutually_exclusive() {
        let config = InstallConfig {
            hostname: Some("server-01".into()),
            hostname_template: Some("srv-{hex:6}".into()),
            ..Default::default()
        };
        let issues = config.validate();
        assert!(issues.iter().any(|i| i.contains("mutually exclusive")));
    }

    #[test]
    fn validate_dhcp_and_template_mutually_exclusive() {
        let config = InstallConfig {
            hostname_from_dhcp: true,
            hostname_template: Some("srv-{hex:6}".into()),
            ..Default::default()
        };
        let issues = config.validate();
        assert!(issues.iter().any(|i| i.contains("mutually exclusive")));
    }

    #[test]
    fn validate_bad_hostname_template() {
        let config = InstallConfig {
            hostname_template: Some("".into()),
            ..Default::default()
        };
        let issues = config.validate();
        assert!(issues.iter().any(|i| i.contains("hostname-template")));
    }

    #[test]
    fn validate_dhcp_on_none_encryption_warns() {
        let config = InstallConfig {
            disk_encryption: Some(DiskEncryption::None),
            hostname_from_dhcp: true,
            ..Default::default()
        };
        let issues = config.validate();
        assert!(
            issues
                .iter()
                .any(|i| i.contains("hostname-from-dhcp has no special effect"))
        );
    }

    #[test]
    fn parse_hostname_from_dhcp() {
        let config = InstallConfig::from_toml(
            r#"
            hostname-from-dhcp = true
        "#,
        )
        .unwrap();
        assert!(config.hostname_from_dhcp);
        assert_eq!(config.hostname, None);
        assert_eq!(config.hostname_template, None);
    }

    #[test]
    fn parse_hostname_template() {
        let config = InstallConfig::from_toml(
            r#"
            hostname-template = "srv-{hex:6}"
        "#,
        )
        .unwrap();
        assert_eq!(config.hostname_template.as_deref(), Some("srv-{hex:6}"));
        assert_eq!(config.hostname, None);
        assert!(!config.hostname_from_dhcp);
    }

    #[test]
    fn has_hostname_config_none() {
        let cfg = InstallConfig::default();
        assert!(!cfg.has_hostname_config());
    }

    #[test]
    fn has_hostname_config_hostname() {
        let cfg = InstallConfig {
            hostname: Some("test".into()),
            ..Default::default()
        };
        assert!(cfg.has_hostname_config());
    }

    #[test]
    fn has_hostname_config_dhcp() {
        let cfg = InstallConfig {
            hostname_from_dhcp: true,
            ..Default::default()
        };
        assert!(cfg.has_hostname_config());
    }

    #[test]
    fn has_hostname_config_template() {
        let cfg = InstallConfig {
            hostname_template: Some("srv-{hex:6}".into()),
            ..Default::default()
        };
        assert!(cfg.has_hostname_config());
    }

    // r[verify installer.mode.interactive+2]
    // r[verify installer.mode.prefilled]
    // r[verify installer.mode.auto+4]
    // r[verify installer.mode.auto-incomplete+3]
    #[test]
    fn operating_mode_display() {
        assert_eq!(OperatingMode::Interactive.to_string(), "interactive");
        assert_eq!(OperatingMode::Prefilled.to_string(), "prefilled");
        assert_eq!(OperatingMode::Auto.to_string(), "automatic");
        assert_eq!(
            OperatingMode::AutoIncomplete {
                missing_disk_encryption: true,
                missing_disk: true,
                missing_hostname: false,
            }
            .to_string(),
            "automatic (incomplete config)"
        );
    }

    // r[verify installer.config.disk-encryption]
    #[test]
    fn disk_encryption_is_encrypted() {
        assert!(DiskEncryption::Tpm.is_encrypted());
        assert!(DiskEncryption::Keyfile.is_encrypted());
        assert!(!DiskEncryption::None.is_encrypted());
    }
}
