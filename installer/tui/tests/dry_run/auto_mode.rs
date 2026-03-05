use predicates::prelude::*;

use super::common::{Fixture, SINGLE_SSD_DEVICE, TWO_DISK_DEVICES, installer};

// r[verify installer.dryrun]
// r[verify installer.dryrun.output]
// r[verify installer.dryrun.schema+2]
// r[verify installer.mode.auto+2]
#[test]
fn auto_full_config_produces_correct_plan() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
        disk = "largest-ssd"

        [firstboot]
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
    assert_eq!(plan["variant"], "metal");
    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
    assert_eq!(plan["disk"]["model"], "Samsung 980 PRO");
    assert_eq!(plan["disk"]["size_bytes"], 1000204886016u64);
    assert_eq!(plan["disk"]["transport"], "NVMe");
    assert!(!plan["disable_tpm"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["hostname"], "server-01");
    assert!(plan["firstboot"]["tailscale_authkey"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["ssh_authorized_keys_count"], 2);
    assert!(plan["config_warnings"].as_array().unwrap().is_empty());
}

// r[verify installer.dryrun.schema+2]
// r[verify installer.config.schema+2]
#[test]
fn auto_disk_path_resolves_correctly() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    let config = f.write_config(
        r#"
        auto = true
        variant = "cloud"
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
    assert_eq!(plan["variant"], "cloud");
    assert_eq!(plan["disk"]["path"], "/dev/sda");
    assert_eq!(plan["disk"]["model"], "WD Blue");
}

// r[verify installer.dryrun.schema+2]
// r[verify installer.config.schema+2]
// r[verify image.tpm.disableable]
#[test]
fn auto_disable_tpm_reflected_in_plan() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
        disk = "largest-ssd"
        disable-tpm = true

        [firstboot]
        hostname = "test-tpm"
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
    assert!(plan["disable_tpm"].as_bool().unwrap());
}

// r[verify installer.dryrun.schema+2]
#[test]
fn auto_cloud_variant_no_firstboot() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "cloud"
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
    assert_eq!(plan["variant"], "cloud");
    assert!(plan["firstboot"].is_null());
    assert!(!plan["disable_tpm"].as_bool().unwrap());
}

// r[verify installer.config.schema+2]
// r[verify image.tpm.disableable]
#[test]
fn auto_disable_tpm_on_cloud_emits_warning() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "cloud"
        disk = "largest-ssd"
        disable-tpm = true
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
            .any(|w| w.as_str().unwrap().contains("disable-tpm")),
        "expected a warning about disable-tpm, got: {warnings:?}"
    );
}

// r[verify installer.config.schema+2]
#[test]
fn auto_bad_hostname_emits_warning() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "cloud"
        disk = "largest-ssd"

        [firstboot]
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

// r[verify installer.mode.auto-incomplete+2]
#[test]
fn auto_incomplete_missing_variant_falls_back() {
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
        .stderr(predicates::str::contains("variant"));

    let plan = f.read_plan();
    assert_eq!(plan["mode"], "auto-incomplete");
}

// r[verify installer.mode.auto-incomplete+2]
#[test]
fn auto_incomplete_missing_disk_falls_back() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
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

// r[verify installer.mode.auto-incomplete+2]
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
        .stderr(predicates::str::contains("variant").and(predicates::str::contains("disk")));

    let plan = f.read_plan();
    assert_eq!(plan["mode"], "auto-incomplete");
}

// r[verify installer.config.schema+2]
#[test]
fn auto_with_minimal_firstboot() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "cloud"
        disk = "largest-ssd"

        [firstboot]
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
    assert_eq!(plan["firstboot"]["hostname"], "just-a-hostname");
    assert!(!plan["firstboot"]["tailscale_authkey"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["ssh_authorized_keys_count"], 0);
}

// r[verify installer.config.schema+2]
#[test]
fn auto_with_only_ssh_keys() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
        disk = "largest-ssd"

        [firstboot]
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
    assert_eq!(plan["firstboot"]["hostname"], "test-host");
    assert!(!plan["firstboot"]["tailscale_authkey"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["ssh_authorized_keys_count"], 3);
}
