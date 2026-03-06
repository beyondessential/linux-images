use predicates::prelude::*;

use super::common::{Fixture, SINGLE_SSD_DEVICE, TWO_DISK_DEVICES, installer};

// r[verify installer.dryrun]
// r[verify installer.dryrun.output]
// r[verify installer.dryrun.schema+5]
// r[verify installer.mode.auto+4]
#[test]
fn auto_full_config_produces_correct_plan() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        hostname = "server-01"
        tailscale-authkey = "tskey-auth-xxxxx"
        ssh-authorized-keys = [
            "ssh-ed25519 AAAA admin@example.com",
            "ssh-rsa BBBB backup@example.com",
        ]
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
    assert_eq!(plan["variant"], "metal");
    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
    assert_eq!(plan["disk"]["model"], "Samsung 980 PRO");
    assert_eq!(plan["disk"]["size_bytes"], 1000204886016u64);
    assert_eq!(plan["disk"]["transport"], "NVMe");
    assert!(!plan["tpm_present"].as_bool().unwrap());
    assert_eq!(plan["install_config"]["hostname"], "server-01");
    assert!(
        plan["install_config"]["tailscale_authkey"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(plan["install_config"]["ssh_authorized_keys_count"], 2);
    assert!(plan["config_warnings"].as_array().unwrap().is_empty());
}

// r[verify installer.dryrun.schema+5]
// r[verify installer.config.schema+4]
#[test]
fn auto_disk_path_resolves_correctly() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "none"
        disk = "/dev/sda"
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
    assert_eq!(plan["disk_encryption"], "none");
    assert_eq!(plan["variant"], "cloud");
    assert_eq!(plan["disk"]["path"], "/dev/sda");
    assert_eq!(plan["disk"]["model"], "WD Blue");
}

// r[verify installer.dryrun.schema+5]
// r[verify installer.config.schema+4]
#[test]
fn auto_keyfile_encryption_produces_metal_variant() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "keyfile"
        disk = "largest-ssd"

        hostname = "test-keyfile"
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
    assert_eq!(plan["disk_encryption"], "keyfile");
    assert_eq!(plan["variant"], "metal");
}

// r[verify installer.dryrun.schema+5]
// r[verify installer.dryrun.fake-tpm]
#[test]
fn auto_fake_tpm_reflected_in_plan() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        hostname = "test-tpm"
    "#,
    );

    installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--fake-tpm",
            "--dry-run",
            "--dry-run-output",
            f.plan_path().to_str().unwrap(),
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let plan = f.read_plan();
    assert!(plan["tpm_present"].as_bool().unwrap());
    assert_eq!(plan["disk_encryption"], "tpm");
}

// r[verify installer.dryrun.schema+5]
#[test]
fn auto_none_encryption_no_install_config() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "none"
        disk = "largest-ssd"
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
    assert_eq!(plan["disk_encryption"], "none");
    assert_eq!(plan["variant"], "cloud");
    assert!(plan["install_config"].is_null());
    assert!(!plan["tpm_present"].as_bool().unwrap());
}

// r[verify installer.config.schema+4]
#[test]
fn auto_bad_hostname_emits_warning() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "none"
        disk = "largest-ssd"

        hostname = "-invalid-"
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
            .contains("must not start or end with a hyphen")),
        "expected a hostname validation warning, got: {warnings:?}"
    );
}

// r[verify installer.mode.auto-incomplete+3]
#[test]
fn auto_incomplete_missing_disk_encryption_falls_back() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk = "largest-ssd"
    "#,
    );
    let script = f.write_script(
        "enter\nenter\nenter\nenter\ntype:h\nenter\nenter\ntab\nenter\nenter\nenter\ntype:yes\nenter\n",
    );
    let timezones = f.write_timezones();

    installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--fake-timezones",
            timezones.to_str().unwrap(),
            "--input-script",
            script.to_str().unwrap(),
            "--dry-run",
            "--dry-run-output",
            f.plan_path().to_str().unwrap(),
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicates::str::contains("disk-encryption"));

    let plan = f.read_plan();
    assert_eq!(plan["mode"], "auto-incomplete");
}

// r[verify installer.mode.auto-incomplete+3]
#[test]
fn auto_incomplete_missing_disk_falls_back() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
    "#,
    );
    let script = f.write_script(
        "enter\nenter\nenter\nenter\ntype:h\nenter\nenter\ntab\nenter\nenter\nenter\ntype:yes\nenter\n",
    );
    let timezones = f.write_timezones();

    installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--fake-timezones",
            timezones.to_str().unwrap(),
            "--input-script",
            script.to_str().unwrap(),
            "--dry-run",
            "--dry-run-output",
            f.plan_path().to_str().unwrap(),
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(predicates::str::contains("disk").and(predicates::str::contains("hostname")));

    let plan = f.read_plan();
    assert_eq!(plan["mode"], "auto-incomplete");
}

// r[verify installer.mode.auto-incomplete+3]
#[test]
fn auto_incomplete_missing_both_falls_back() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config("auto = true\n");
    let script = f.write_script(
        "enter\nenter\nenter\nenter\ntype:h\nenter\nenter\ntab\nenter\nenter\nenter\ntype:yes\nenter\n",
    );
    let timezones = f.write_timezones();

    installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--fake-timezones",
            timezones.to_str().unwrap(),
            "--input-script",
            script.to_str().unwrap(),
            "--dry-run",
            "--dry-run-output",
            f.plan_path().to_str().unwrap(),
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stderr(
            predicates::str::contains("disk-encryption").and(predicates::str::contains("disk")),
        );

    let plan = f.read_plan();
    assert_eq!(plan["mode"], "auto-incomplete");
}

// r[verify installer.config.schema+4]
#[test]
fn auto_with_minimal_install_config() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "none"
        disk = "largest-ssd"

        hostname = "just-a-hostname"
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
    assert_eq!(plan["install_config"]["hostname"], "just-a-hostname");
    assert!(
        !plan["install_config"]["tailscale_authkey"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(plan["install_config"]["ssh_authorized_keys_count"], 0);
}

// r[verify installer.config.schema+4]
#[test]
fn auto_with_only_ssh_keys() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        hostname = "test-host"
        ssh-authorized-keys = [
            "ssh-ed25519 AAAA k1",
            "ssh-ed25519 BBBB k2",
            "ssh-rsa CCCC k3",
        ]
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
    assert_eq!(plan["install_config"]["hostname"], "test-host");
    assert!(
        !plan["install_config"]["tailscale_authkey"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(plan["install_config"]["ssh_authorized_keys_count"], 3);
}
