use serde_json::Value;

use super::common::{Fixture, SINGLE_SSD_DEVICE, installer};

// r[verify installer.config.hostname]
// r[verify installer.dryrun.schema+6]
#[test]
fn auto_encrypted_hostname_from_dhcp() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        hostname-from-dhcp = true
    "#,
    );

    installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--dry-run-output",
            f.plan_path().to_str().unwrap(),
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let plan = f.read_plan();
    assert_eq!(plan["mode"], "auto");
    assert_eq!(plan["disk_encryption"], "tpm");
    assert_eq!(
        plan["install_config"]["hostname"], "dhcp",
        "hostname should be the sentinel string 'dhcp'"
    );
    assert!(
        !plan["install_config"]["hostname_from_template"]
            .as_bool()
            .unwrap()
    );
}

// r[verify installer.config.hostname-template]
// r[verify installer.dryrun.schema+6]
#[test]
fn auto_encrypted_hostname_template() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        hostname-template = "test-{hex:6}"
    "#,
    );

    installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--dry-run-output",
            f.plan_path().to_str().unwrap(),
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let plan = f.read_plan();
    assert_eq!(plan["mode"], "auto");
    assert_eq!(plan["disk_encryption"], "tpm");

    let hostname = plan["install_config"]["hostname"].as_str().unwrap();
    assert!(
        hostname.starts_with("test-"),
        "resolved hostname should start with 'test-', got: {hostname}"
    );
    assert_eq!(
        hostname.len(),
        11,
        "resolved hostname should be 11 chars (test- + 6 hex), got: {hostname}"
    );
    let hex_part = &hostname[5..];
    assert!(
        hex_part.chars().all(|c| c.is_ascii_hexdigit()),
        "hex portion should be valid hex, got: {hex_part}"
    );
    assert!(
        plan["install_config"]["hostname_from_template"]
            .as_bool()
            .unwrap(),
        "hostname_from_template should be true"
    );
}

// r[verify installer.config.hostname-template]
#[test]
fn auto_encrypted_hostname_template_num() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        hostname-template = "node-{num:4}"
    "#,
    );

    installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--dry-run-output",
            f.plan_path().to_str().unwrap(),
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let plan = f.read_plan();
    let hostname = plan["install_config"]["hostname"].as_str().unwrap();
    assert!(
        hostname.starts_with("node-"),
        "resolved hostname should start with 'node-', got: {hostname}"
    );
    assert_eq!(hostname.len(), 9);
    let num_part = &hostname[5..];
    assert!(
        num_part.chars().all(|c| c.is_ascii_digit()),
        "num portion should be all digits, got: {num_part}"
    );
}

// r[verify installer.config.hostname]
#[test]
fn auto_hostname_and_dhcp_mutually_exclusive() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        hostname = "server-01"
        hostname-from-dhcp = true
    "#,
    );

    installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--dry-run-output",
            f.plan_path().to_str().unwrap(),
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let plan = f.read_plan();
    let warnings = plan["config_warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap().contains("mutually exclusive")),
        "should warn about mutually exclusive hostname fields, got: {warnings:?}"
    );
}

// r[verify installer.config.hostname]
#[test]
fn auto_hostname_and_template_mutually_exclusive() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        hostname = "server-01"
        hostname-template = "srv-{hex:6}"
    "#,
    );

    installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--dry-run-output",
            f.plan_path().to_str().unwrap(),
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let plan = f.read_plan();
    let warnings = plan["config_warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap().contains("mutually exclusive")),
        "should warn about mutually exclusive hostname fields, got: {warnings:?}"
    );
}

// r[verify installer.config.hostname]
#[test]
fn auto_dhcp_always_emits_warning() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "none"
        disk = "largest-ssd"

        hostname-from-dhcp = true
    "#,
    );

    installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--dry-run-output",
            f.plan_path().to_str().unwrap(),
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let plan = f.read_plan();
    let warnings = plan["config_warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| w
            .as_str()
            .unwrap()
            .contains("hostname-from-dhcp has no special effect")),
        "should warn about redundant hostname-from-dhcp, got: {warnings:?}"
    );
}

// r[verify installer.config.hostname-template]
#[test]
fn auto_invalid_hostname_template_emits_warning() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        hostname-template = "no-placeholder"
    "#,
    );

    installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--dry-run-output",
            f.plan_path().to_str().unwrap(),
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure();
}

// r[verify installer.tui.hostname+6]
#[test]
fn scripted_encrypted_dhcp_toggle_produces_dhcp_sentinel() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let script = f.write_script(
        "# Welcome -> NetworkConfig
enter
# NetworkConfig -> DiskSelection
enter
# DiskSelection -> DiskEncryptionScreen (default Keyfile)
enter
# DiskEncryptionScreen -> Hostname selector
enter
# Hostname selector: Network-assigned is default, Enter -> Login
enter
# Login: type password
type:pw
enter
type:pw
enter
# Timezone: accept default
enter
# NetworkResults
enter
type:yes
enter
",
    );

    installer()
        .args([
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--dry-run-output",
            f.plan_path().to_str().unwrap(),
            "--input-script",
            script.to_str().unwrap(),
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let plan = f.read_plan();
    assert_eq!(
        plan["install_config"]["hostname"], "dhcp",
        "hostname should be sentinel 'dhcp' when DHCP toggle is active"
    );
}

// r[verify installer.config.hostname-template]
#[test]
fn auto_encrypted_hostname_template_two_runs_differ() {
    let f = Fixture::new();
    let devices_path = f.write_devices(SINGLE_SSD_DEVICE);
    let config_path = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        hostname-template = "uniq-{hex:8}"
    "#,
    );

    let plan1_path = f.path("plan1.json");
    let plan2_path = f.path("plan2.json");

    installer()
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "--fake-devices",
            devices_path.to_str().unwrap(),
            "--dry-run",
            "--dry-run-output",
            plan1_path.to_str().unwrap(),
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    installer()
        .args([
            "--config",
            config_path.to_str().unwrap(),
            "--fake-devices",
            devices_path.to_str().unwrap(),
            "--dry-run",
            "--dry-run-output",
            plan2_path.to_str().unwrap(),
            "--log",
            f.path("installer2.log").to_str().unwrap(),
        ])
        .assert()
        .success();

    let p1: Value = serde_json::from_str(&std::fs::read_to_string(&plan1_path).unwrap()).unwrap();
    let p2: Value = serde_json::from_str(&std::fs::read_to_string(&plan2_path).unwrap()).unwrap();

    let h1 = p1["install_config"]["hostname"].as_str().unwrap();
    let h2 = p2["install_config"]["hostname"].as_str().unwrap();

    assert_ne!(
        h1, h2,
        "two template resolutions should produce different hostnames (collision extremely unlikely with 8 hex chars)"
    );
}
