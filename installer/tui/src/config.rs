use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize, Serializer};

use crate::hostname_template;

// r[impl installer.config.format]
// r[impl installer.config.auto+2]
// r[impl installer.config.disk-encryption+2]
// r[impl installer.config.disk]
// r[impl installer.config.copy-install-log]
// r[impl installer.config.hostname+2]
// r[impl installer.config.tailscale-authkey+3]
// r[impl installer.config.ssh-authorized-keys+2]
// r[impl installer.config.password]
// r[impl installer.config.timezone]
// r[impl installer.config.recovery-passphrase]
// r[impl installer.config.save-recovery-keys]
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
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

    #[serde(default = "default_true", rename = "hostname-from-dhcp")]
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

    #[serde(default, rename = "recovery-passphrase")]
    pub recovery_passphrase: Option<String>,

    #[serde(default, rename = "save-recovery-keys")]
    pub save_recovery_keys: bool,

    // r[impl installer.config.network-mode]
    #[serde(default, rename = "network-mode")]
    pub network_mode: Option<NetworkMode>,

    // r[impl installer.config.network-static]
    #[serde(default, rename = "network-interface")]
    pub network_interface: Option<String>,

    #[serde(default, rename = "network-ip")]
    pub network_ip: Option<String>,

    #[serde(default, rename = "network-gateway")]
    pub network_gateway: Option<String>,

    #[serde(default, rename = "network-dns")]
    pub network_dns: Option<String>,

    #[serde(default, rename = "network-domain")]
    pub network_domain: Option<String>,

    // r[impl installer.config.iso-network-mode]
    #[serde(default, rename = "iso-network-mode")]
    pub iso_network_mode: Option<NetworkMode>,

    #[serde(default, rename = "iso-network-interface")]
    pub iso_network_interface: Option<String>,

    #[serde(default, rename = "iso-network-ip")]
    pub iso_network_ip: Option<String>,

    #[serde(default, rename = "iso-network-gateway")]
    pub iso_network_gateway: Option<String>,

    #[serde(default, rename = "iso-network-dns")]
    pub iso_network_dns: Option<String>,

    #[serde(default, rename = "iso-network-domain")]
    pub iso_network_domain: Option<String>,
}

const fn default_true() -> bool {
    true
}

impl Default for InstallConfig {
    fn default() -> Self {
        Self {
            auto: false,
            disk_encryption: None,
            disk: None,
            copy_install_log: None,
            hostname: None,
            hostname_from_dhcp: true,
            hostname_template: None,
            tailscale_authkey: None,
            ssh_authorized_keys: Vec::new(),
            password: None,
            password_hash: None,
            timezone: None,
            recovery_passphrase: None,
            save_recovery_keys: false,
            network_mode: None,
            network_interface: None,
            network_ip: None,
            network_gateway: None,
            network_dns: None,
            network_domain: None,
            iso_network_mode: None,
            iso_network_interface: None,
            iso_network_ip: None,
            iso_network_gateway: None,
            iso_network_dns: None,
            iso_network_domain: None,
        }
    }
}

const RECOVERY_PASSPHRASE_MIN_LEN: usize = 25;

/// Validate that a recovery passphrase meets requirements: at least 25
/// characters, only printable ASCII (no whitespace). Returns `Ok(())` or
/// an error message.
// r[impl installer.config.recovery-passphrase]
pub fn validate_recovery_passphrase(passphrase: &str) -> std::result::Result<(), String> {
    if passphrase.len() < RECOVERY_PASSPHRASE_MIN_LEN {
        return Err(format!(
            "recovery passphrase must be at least {RECOVERY_PASSPHRASE_MIN_LEN} characters, got {}",
            passphrase.len()
        ));
    }
    if let Some(pos) = passphrase.chars().position(|c| {
        // printable ASCII excluding whitespace: '!' (0x21) through '~' (0x7E)
        !matches!(c, '!'..='~')
    }) {
        let bad = passphrase.chars().nth(pos).unwrap();
        return Err(format!(
            "recovery passphrase contains invalid character {bad:?} at position {pos}; \
             only printable ASCII (no whitespace) is allowed"
        ));
    }
    Ok(())
}

/// Network configuration mode for a pane (ISO or target).
// r[impl installer.config.network-mode]
// r[impl installer.config.iso-network-mode]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkMode {
    Dhcp,
    StaticIp,
    Ipv6Slaac,
    Offline,
}

impl fmt::Display for NetworkMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NetworkMode::Dhcp => write!(f, "dhcp"),
            NetworkMode::StaticIp => write!(f, "static"),
            NetworkMode::Ipv6Slaac => write!(f, "ipv6-slaac"),
            NetworkMode::Offline => write!(f, "offline"),
        }
    }
}

impl<'de> Deserialize<'de> for NetworkMode {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "dhcp" => Ok(NetworkMode::Dhcp),
            "static" => Ok(NetworkMode::StaticIp),
            "ipv6-slaac" => Ok(NetworkMode::Ipv6Slaac),
            "offline" => Ok(NetworkMode::Offline),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["dhcp", "static", "ipv6-slaac", "offline"],
            )),
        }
    }
}

impl Serialize for NetworkMode {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiskEncryption {
    Tpm,
    Keyfile,
    None,
}

impl DiskEncryption {
    pub fn image_variant_str(self) -> &'static str {
        match self {
            DiskEncryption::Tpm => "luks-tpm",
            DiskEncryption::Keyfile => "luks-keyfile",
            DiskEncryption::None => "plain",
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

impl Serialize for DiskEncryption {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiskSelector {
    Path(PathBuf),
    Strategy(DiskStrategy),
}

impl Serialize for DiskSelector {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
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
        self.hostname.is_some() || self.hostname_template.is_some() || self.hostname_from_dhcp
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
// r[impl installer.mode.auto+5]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperatingMode {
    Interactive,
    Prefilled,
    Auto,
}

impl fmt::Display for OperatingMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OperatingMode::Interactive => write!(f, "interactive"),
            OperatingMode::Prefilled => write!(f, "prefilled"),
            OperatingMode::Auto => write!(f, "automatic"),
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

    // r[impl installer.mode.auto+5]
    // r[impl installer.config.auto+2]
    pub fn mode(&self) -> OperatingMode {
        if self.auto {
            OperatingMode::Auto
        } else {
            OperatingMode::Prefilled
        }
    }

    /// Hard validation: returns `Err` for problems that must prevent the
    /// install from proceeding.
    pub fn validate_hard(&self) -> Result<()> {
        // r[impl installer.config.recovery-passphrase]
        if let Some(ref pp) = self.recovery_passphrase {
            validate_recovery_passphrase(pp).map_err(|e| anyhow::anyhow!("config error: {e}"))?;
        }
        Ok(())
    }

    pub fn validate(&self) -> Vec<String> {
        let mut issues = Vec::new();

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

        for (i, key) in self.ssh_authorized_keys.iter().enumerate() {
            if key.trim().is_empty() {
                issues.push(format!("ssh-authorized-keys[{i}] is empty"));
            }
        }

        if self.password.is_some() && self.password_hash.is_some() {
            issues.push("password and password-hash are mutually exclusive".into());
        }

        // r[impl installer.config.network-static]
        validate_network_static_fields(
            "network",
            self.network_mode,
            self.network_ip.as_deref(),
            self.network_gateway.as_deref(),
            &mut issues,
        );

        // r[impl installer.config.iso-network-mode]
        validate_network_static_fields(
            "iso-network",
            self.iso_network_mode,
            self.iso_network_ip.as_deref(),
            self.iso_network_gateway.as_deref(),
            &mut issues,
        );

        issues
    }
}

/// Validate that when a network mode is "static", the required ip and gateway
/// fields are present. `prefix` is either `"network"` or `"iso-network"`.
fn validate_network_static_fields(
    prefix: &str,
    mode: Option<NetworkMode>,
    ip: Option<&str>,
    gateway: Option<&str>,
    issues: &mut Vec<String>,
) {
    if mode == Some(NetworkMode::StaticIp) {
        if ip.is_none_or(|s| s.trim().is_empty()) {
            issues.push(format!(
                "{prefix}-ip is required when {prefix}-mode is \"static\""
            ));
        }
        if gateway.is_none_or(|s| s.trim().is_empty()) {
            issues.push(format!(
                "{prefix}-gateway is required when {prefix}-mode is \"static\""
            ));
        }
    }
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
    // r[verify installer.config.auto+2]
    // r[verify installer.config.disk-encryption+2]
    // r[verify installer.config.disk]
    // r[verify installer.config.hostname+2]
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
        assert_eq!(config.recovery_passphrase, None);
        assert!(!config.save_recovery_keys);
        assert_eq!(config.network_mode, None);
        assert_eq!(config.network_interface, None);
        assert_eq!(config.network_ip, None);
        assert_eq!(config.network_gateway, None);
        assert_eq!(config.network_dns, None);
        assert_eq!(config.network_domain, None);
        assert_eq!(config.iso_network_mode, None);
    }

    // r[verify installer.config.disk-encryption+2]
    #[test]
    fn parse_disk_encryption_variants() {
        let tpm = InstallConfig::from_toml(r#"disk-encryption = "tpm""#).unwrap();
        assert_eq!(tpm.disk_encryption, Some(DiskEncryption::Tpm));

        let keyfile = InstallConfig::from_toml(r#"disk-encryption = "keyfile""#).unwrap();
        assert_eq!(keyfile.disk_encryption, Some(DiskEncryption::Keyfile));

        let none = InstallConfig::from_toml(r#"disk-encryption = "none""#).unwrap();
        assert_eq!(none.disk_encryption, Some(DiskEncryption::None));
    }

    // r[verify installer.config.disk-encryption+2]
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

    // r[verify installer.mode.auto+5]
    // r[verify installer.config.auto+2]
    #[test]
    fn mode_auto_with_all_fields() {
        let config = InstallConfig {
            auto: true,
            disk_encryption: Some(DiskEncryption::Tpm),
            disk: Some(DiskSelector::Strategy(DiskStrategy::LargestSsd)),
            hostname: Some("my-server".into()),
            ..Default::default()
        };
        assert_eq!(config.mode(), OperatingMode::Auto);
    }

    // r[verify installer.mode.auto+5]
    // r[verify installer.config.auto+2]
    #[test]
    fn mode_auto_with_no_optional_fields() {
        let config = InstallConfig {
            auto: true,
            ..Default::default()
        };
        assert_eq!(config.mode(), OperatingMode::Auto);
    }

    // r[verify installer.mode.auto+5]
    // r[verify installer.config.auto+2]
    #[test]
    fn mode_auto_without_disk_encryption() {
        let config = InstallConfig {
            auto: true,
            disk: Some(DiskSelector::Strategy(DiskStrategy::LargestSsd)),
            ..Default::default()
        };
        assert_eq!(config.mode(), OperatingMode::Auto);
    }

    // r[verify installer.mode.auto+5]
    // r[verify installer.config.auto+2]
    #[test]
    fn mode_auto_without_disk() {
        let config = InstallConfig {
            auto: true,
            disk_encryption: Some(DiskEncryption::Keyfile),
            ..Default::default()
        };
        assert_eq!(config.mode(), OperatingMode::Auto);
    }

    // r[verify installer.mode.auto+5]
    // r[verify installer.config.auto+2]
    #[test]
    fn mode_auto_encrypted_without_hostname() {
        let config = InstallConfig {
            auto: true,
            disk_encryption: Some(DiskEncryption::Tpm),
            disk: Some(DiskSelector::Strategy(DiskStrategy::LargestSsd)),
            ..Default::default()
        };
        assert_eq!(config.mode(), OperatingMode::Auto);
    }

    // r[verify installer.config.hostname+2]
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

    // r[verify installer.config.hostname+2]
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
            disk_encryption: Some(DiskEncryption::Tpm),
            disk: Some(DiskSelector::Strategy(DiskStrategy::LargestSsd)),
            hostname: Some("server-01".into()),
            tailscale_authkey: Some("tskey-auth-xxxxx".into()),
            ssh_authorized_keys: vec!["ssh-ed25519 AAAA admin@example.com".into()],
            password: Some("testpass".into()),
            hostname_from_dhcp: true,
            ..Default::default()
        };
        let issues = config.validate();
        assert!(issues.is_empty(), "unexpected issues: {issues:?}");
    }

    // r[verify installer.config.disk-encryption+2]
    #[test]
    fn disk_encryption_display() {
        assert_eq!(DiskEncryption::Tpm.to_string(), "tpm");
        assert_eq!(DiskEncryption::Keyfile.to_string(), "keyfile");
        assert_eq!(DiskEncryption::None.to_string(), "none");
    }

    // r[verify installer.config.disk-encryption+2]
    #[test]
    fn disk_encryption_image_variant_str() {
        assert_eq!(DiskEncryption::Tpm.image_variant_str(), "luks-tpm");
        assert_eq!(DiskEncryption::Keyfile.image_variant_str(), "luks-keyfile");
        assert_eq!(DiskEncryption::None.image_variant_str(), "plain");
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

    // r[verify installer.config.hostname+2]
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

    // r[verify installer.config.hostname+2]
    #[test]
    fn hostname_from_dhcp_defaults_to_true() {
        let config = InstallConfig::default();
        assert!(config.hostname_from_dhcp);
    }

    // r[verify installer.config.hostname+2]
    #[test]
    fn hostname_priority_hostname_wins() {
        let config = InstallConfig {
            hostname: Some("server-01".into()),
            hostname_template: Some("srv-{hex:6}".into()),
            hostname_from_dhcp: true,
            ..Default::default()
        };
        let issues = config.validate();
        assert!(
            !issues.iter().any(|i| i.contains("mutually exclusive")),
            "should not report mutual exclusivity: {issues:?}"
        );
    }

    // r[verify installer.config.hostname+2]
    #[test]
    fn hostname_priority_template_over_dhcp() {
        let config = InstallConfig {
            hostname_template: Some("srv-{hex:6}".into()),
            hostname_from_dhcp: true,
            ..Default::default()
        };
        let issues = config.validate();
        assert!(
            !issues.iter().any(|i| i.contains("mutually exclusive")),
            "should not report mutual exclusivity: {issues:?}"
        );
    }

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

    // r[verify installer.config.hostname+2]
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
    fn validate_dhcp_default_no_warnings() {
        let config = InstallConfig {
            disk_encryption: Some(DiskEncryption::Tpm),
            ..Default::default()
        };
        let issues = config.validate();
        assert!(
            !issues.iter().any(|i| i.contains("hostname-from-dhcp")),
            "should not warn about hostname-from-dhcp: {issues:?}"
        );
    }

    #[test]
    fn parse_hostname_from_dhcp_explicit_true() {
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

    // r[verify installer.config.hostname+2]
    #[test]
    fn parse_hostname_from_dhcp_explicit_false() {
        let config = InstallConfig::from_toml(
            r#"
            hostname-from-dhcp = false
        "#,
        )
        .unwrap();
        assert!(!config.hostname_from_dhcp);
    }

    // r[verify installer.config.hostname+2]
    #[test]
    fn parse_hostname_from_dhcp_defaults_true() {
        let config = InstallConfig::from_toml("").unwrap();
        assert!(config.hostname_from_dhcp);
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
        assert!(config.hostname_from_dhcp);
    }

    #[test]
    fn has_hostname_config_default_is_true() {
        let cfg = InstallConfig::default();
        assert!(cfg.has_hostname_config());
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
    // r[verify installer.mode.auto+5]
    #[test]
    fn operating_mode_display() {
        assert_eq!(OperatingMode::Interactive.to_string(), "interactive");
        assert_eq!(OperatingMode::Prefilled.to_string(), "prefilled");
        assert_eq!(OperatingMode::Auto.to_string(), "automatic");
    }

    // r[verify installer.config.network-mode]
    #[test]
    fn parse_network_mode_variants() {
        for (input, expected) in [
            ("dhcp", NetworkMode::Dhcp),
            ("static", NetworkMode::StaticIp),
            ("ipv6-slaac", NetworkMode::Ipv6Slaac),
            ("offline", NetworkMode::Offline),
        ] {
            let config = InstallConfig::from_toml(&format!(r#"network-mode = "{input}""#)).unwrap();
            assert_eq!(config.network_mode, Some(expected));
        }
    }

    // r[verify installer.config.network-mode]
    #[test]
    fn parse_invalid_network_mode_rejected() {
        let result = InstallConfig::from_toml(r#"network-mode = "bad""#);
        assert!(result.is_err());
    }

    // r[verify installer.config.network-static]
    #[test]
    fn parse_network_static_fields() {
        let toml = r#"
            network-mode = "static"
            network-interface = "enp0s3"
            network-ip = "192.168.1.10/24"
            network-gateway = "192.168.1.1"
            network-dns = "8.8.8.8, 1.1.1.1"
            network-domain = "example.com"
        "#;
        let config = InstallConfig::from_toml(toml).unwrap();
        assert_eq!(config.network_mode, Some(NetworkMode::StaticIp));
        assert_eq!(config.network_interface.as_deref(), Some("enp0s3"));
        assert_eq!(config.network_ip.as_deref(), Some("192.168.1.10/24"));
        assert_eq!(config.network_gateway.as_deref(), Some("192.168.1.1"));
        assert_eq!(config.network_dns.as_deref(), Some("8.8.8.8, 1.1.1.1"));
        assert_eq!(config.network_domain.as_deref(), Some("example.com"));
    }

    // r[verify installer.config.network-static]
    #[test]
    fn validate_static_network_missing_ip() {
        let config = InstallConfig {
            network_mode: Some(NetworkMode::StaticIp),
            network_gateway: Some("192.168.1.1".into()),
            ..Default::default()
        };
        let issues = config.validate();
        assert!(
            issues.iter().any(|i| i.contains("network-ip is required")),
            "expected network-ip required warning, got: {issues:?}"
        );
    }

    // r[verify installer.config.network-static]
    #[test]
    fn validate_static_network_missing_gateway() {
        let config = InstallConfig {
            network_mode: Some(NetworkMode::StaticIp),
            network_ip: Some("192.168.1.10/24".into()),
            ..Default::default()
        };
        let issues = config.validate();
        assert!(
            issues
                .iter()
                .any(|i| i.contains("network-gateway is required")),
            "expected network-gateway required warning, got: {issues:?}"
        );
    }

    // r[verify installer.config.network-static]
    #[test]
    fn validate_static_network_complete_is_clean() {
        let config = InstallConfig {
            network_mode: Some(NetworkMode::StaticIp),
            network_ip: Some("192.168.1.10/24".into()),
            network_gateway: Some("192.168.1.1".into()),
            ..Default::default()
        };
        let issues = config.validate();
        assert!(
            issues.is_empty(),
            "expected no validation issues, got: {issues:?}"
        );
    }

    // r[verify installer.config.iso-network-mode]
    #[test]
    fn parse_iso_network_mode() {
        let toml = r#"
            iso-network-mode = "static"
            iso-network-interface = "enp0s3"
            iso-network-ip = "10.0.0.5/24"
            iso-network-gateway = "10.0.0.1"
            iso-network-dns = "1.1.1.1"
            iso-network-domain = "test.local"
        "#;
        let config = InstallConfig::from_toml(toml).unwrap();
        assert_eq!(config.iso_network_mode, Some(NetworkMode::StaticIp));
        assert_eq!(config.iso_network_interface.as_deref(), Some("enp0s3"));
        assert_eq!(config.iso_network_ip.as_deref(), Some("10.0.0.5/24"));
        assert_eq!(config.iso_network_gateway.as_deref(), Some("10.0.0.1"));
        assert_eq!(config.iso_network_dns.as_deref(), Some("1.1.1.1"));
        assert_eq!(config.iso_network_domain.as_deref(), Some("test.local"));
    }

    // r[verify installer.config.iso-network-mode]
    #[test]
    fn validate_iso_static_network_missing_fields() {
        let config = InstallConfig {
            iso_network_mode: Some(NetworkMode::StaticIp),
            ..Default::default()
        };
        let issues = config.validate();
        assert!(
            issues
                .iter()
                .any(|i| i.contains("iso-network-ip is required")),
            "expected iso-network-ip required warning, got: {issues:?}"
        );
        assert!(
            issues
                .iter()
                .any(|i| i.contains("iso-network-gateway is required")),
            "expected iso-network-gateway required warning, got: {issues:?}"
        );
    }

    // r[verify installer.config.network-mode]
    #[test]
    fn network_mode_display() {
        assert_eq!(NetworkMode::Dhcp.to_string(), "dhcp");
        assert_eq!(NetworkMode::StaticIp.to_string(), "static");
        assert_eq!(NetworkMode::Ipv6Slaac.to_string(), "ipv6-slaac");
        assert_eq!(NetworkMode::Offline.to_string(), "offline");
    }

    // r[verify installer.config.network-mode]
    #[test]
    fn dhcp_network_mode_has_no_static_issues() {
        let config = InstallConfig {
            network_mode: Some(NetworkMode::Dhcp),
            ..Default::default()
        };
        let issues = config.validate();
        assert!(
            !issues.iter().any(|i| i.contains("network-ip")),
            "dhcp should not require network-ip, got: {issues:?}"
        );
    }

    #[test]
    fn disk_encryption_is_encrypted() {
        assert!(DiskEncryption::Tpm.is_encrypted());
        assert!(DiskEncryption::Keyfile.is_encrypted());
        assert!(!DiskEncryption::None.is_encrypted());
    }

    // r[verify installer.config.recovery-passphrase]
    #[test]
    fn parse_recovery_passphrase() {
        let config = InstallConfig::from_toml(
            r#"
            recovery-passphrase = "MyS3cure!Passphrase#12345"
        "#,
        )
        .unwrap();
        assert_eq!(
            config.recovery_passphrase.as_deref(),
            Some("MyS3cure!Passphrase#12345")
        );
    }

    // r[verify installer.config.recovery-passphrase]
    #[test]
    fn validate_recovery_passphrase_too_short() {
        let pp = "short!";
        let result = validate_recovery_passphrase(pp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("at least 25 characters"));
    }

    // r[verify installer.config.recovery-passphrase]
    #[test]
    fn validate_recovery_passphrase_with_whitespace() {
        let pp = "this has spaces and is long enough!!";
        let result = validate_recovery_passphrase(pp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid character"));
    }

    // r[verify installer.config.recovery-passphrase]
    #[test]
    fn validate_recovery_passphrase_with_non_ascii() {
        let pp = "abcdefghijklmnopqrstuvwx\u{00e9}";
        let result = validate_recovery_passphrase(pp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid character"));
    }

    // r[verify installer.config.recovery-passphrase]
    #[test]
    fn validate_recovery_passphrase_valid() {
        let pp = "Correct-Horse-Battery-Staple!1";
        let result = validate_recovery_passphrase(pp);
        assert!(result.is_ok());
    }

    // r[verify installer.config.recovery-passphrase]
    #[test]
    fn validate_recovery_passphrase_exactly_25_chars() {
        let pp = "abcdefghij!@#$%^&*()12345";
        assert_eq!(pp.len(), 25);
        let result = validate_recovery_passphrase(pp);
        assert!(result.is_ok());
    }

    // r[verify installer.config.recovery-passphrase]
    #[test]
    fn validate_hard_rejects_bad_passphrase() {
        let config = InstallConfig {
            recovery_passphrase: Some("tooshort".into()),
            ..Default::default()
        };
        assert!(config.validate_hard().is_err());
    }

    // r[verify installer.config.recovery-passphrase]
    #[test]
    fn validate_hard_accepts_good_passphrase() {
        let config = InstallConfig {
            recovery_passphrase: Some("Correct-Horse-Battery-Staple!1".into()),
            ..Default::default()
        };
        assert!(config.validate_hard().is_ok());
    }

    // r[verify installer.config.recovery-passphrase]
    #[test]
    fn validate_hard_accepts_no_passphrase() {
        let config = InstallConfig::default();
        assert!(config.validate_hard().is_ok());
    }

    // r[verify installer.config.save-recovery-keys]
    #[test]
    fn parse_save_recovery_keys() {
        let config = InstallConfig::from_toml(
            r#"
            save-recovery-keys = true
        "#,
        )
        .unwrap();
        assert!(config.save_recovery_keys);
    }

    // r[verify installer.config.save-recovery-keys]
    #[test]
    fn save_recovery_keys_defaults_false() {
        let config = InstallConfig::from_toml("").unwrap();
        assert!(!config.save_recovery_keys);
    }

    // r[verify installer.config.template]
    #[test]
    fn template_contains_all_config_fields() {
        // Construct a fully-populated InstallConfig so serialization emits every key.
        let full = InstallConfig {
            auto: true,
            disk_encryption: Some(DiskEncryption::Tpm),
            disk: Some(DiskSelector::Strategy(DiskStrategy::Largest)),
            copy_install_log: Some(true),
            hostname: Some("test".into()),
            hostname_from_dhcp: true,
            hostname_template: Some("srv-{hex:4}".into()),
            tailscale_authkey: Some("tskey-auth-xxx".into()),
            ssh_authorized_keys: vec!["ssh-ed25519 AAAA test".into()],
            password: Some("pass".into()),
            password_hash: Some("$6$hash".into()),
            timezone: Some("UTC".into()),
            recovery_passphrase: Some("a]9Kx#mP2vL!nQ7wR4jH6dT0y".into()),
            save_recovery_keys: true,
            network_mode: Some(NetworkMode::StaticIp),
            network_interface: Some("enp0s3".into()),
            network_ip: Some("192.168.1.10/24".into()),
            network_gateway: Some("192.168.1.1".into()),
            network_dns: Some("8.8.8.8".into()),
            network_domain: Some("example.com".into()),
            iso_network_mode: Some(NetworkMode::Dhcp),
            iso_network_interface: Some("enp0s3".into()),
            iso_network_ip: Some("10.0.0.5/24".into()),
            iso_network_gateway: Some("10.0.0.1".into()),
            iso_network_dns: Some("1.1.1.1".into()),
            iso_network_domain: Some("test.local".into()),
        };

        let toml_value = toml::Value::try_from(&full).expect("failed to serialize InstallConfig");
        let keys: Vec<&str> = toml_value
            .as_table()
            .expect("serialized InstallConfig is not a table")
            .keys()
            .map(|k| k.as_str())
            .collect();

        let template_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../iso/bes-install.toml.template");
        let template = std::fs::read_to_string(&template_path).unwrap_or_else(|e| {
            panic!(
                "failed to read template at {}: {e}",
                template_path.display()
            )
        });

        let mut missing = Vec::new();
        for key in &keys {
            // Look for the key as a commented-out TOML entry (e.g. "# key = " or "# key = [")
            let pattern = format!("# {key} = ");
            if !template.contains(&pattern) {
                missing.push(*key);
            }
        }

        assert!(
            missing.is_empty(),
            "BESCONF template is missing entries for these InstallConfig fields: {missing:?}\n\
             Update iso/bes-install.toml.template to include commented-out entries for each field."
        );
    }
}
