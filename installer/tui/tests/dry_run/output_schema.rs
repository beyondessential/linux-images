use serde_json::Value;

use super::common::{Fixture, SINGLE_SSD_DEVICE, TWO_DISK_DEVICES, installer};

// r[verify installer.dryrun.output]
#[test]
fn dry_run_output_to_file() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        hostname = "test-host"
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
        .success()
        .stdout(predicates::str::is_empty());

    let plan = f.read_plan();
    assert_eq!(plan["mode"], "auto");
}

// r[verify installer.dryrun.output]
#[test]
fn dry_run_output_to_stdout() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        hostname = "test-host"
    "#,
    );

    let output = installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let plan: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(plan["mode"], "auto");
    assert_eq!(plan["disk_encryption"], "tpm");
}

// r[verify installer.dryrun.schema+5]
#[test]
fn plan_contains_all_required_fields() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        hostname = "test-box"
        tailscale-authkey = "tskey-auth-xxx"
        ssh-authorized-keys = ["ssh-ed25519 AAAA k1"]
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
    let obj = plan.as_object().unwrap();

    let required_top = [
        "mode",
        "disk_encryption",
        "variant",
        "disk",
        "tpm_present",
        "install_config",
        "manifest_path",
        "copy_install_log",
        "config_warnings",
    ];
    for key in &required_top {
        assert!(obj.contains_key(*key), "missing top-level key: {key}");
    }

    let disk = plan["disk"].as_object().unwrap();
    for key in &["path", "model", "size_bytes", "transport"] {
        assert!(disk.contains_key(*key), "missing disk key: {key}");
    }

    let fb = plan["install_config"].as_object().unwrap();
    for key in &["hostname", "tailscale_authkey", "ssh_authorized_keys_count"] {
        assert!(fb.contains_key(*key), "missing install_config key: {key}");
    }
}

// r[verify installer.dryrun.schema+5]
#[test]
fn plan_tailscale_authkey_is_bool_not_string() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        hostname = "test-host"
        tailscale-authkey = "tskey-auth-secret-should-not-appear"
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
    assert!(plan["install_config"]["tailscale_authkey"].is_boolean());
    assert!(
        plan["install_config"]["tailscale_authkey"]
            .as_bool()
            .unwrap()
    );

    let raw = std::fs::read_to_string(f.plan_path()).unwrap();
    assert!(
        !raw.contains("tskey-auth-secret-should-not-appear"),
        "authkey secret leaked into plan output"
    );
}

// r[verify installer.dryrun]
#[test]
fn dry_run_without_script_emits_initial_state() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    let config = f.write_config(
        r#"
        disk-encryption = "none"
        disk = "/dev/sda"

        hostname = "pre-host"
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
    assert_eq!(plan["mode"], "prefilled");
    assert_eq!(plan["disk_encryption"], "none");
    assert_eq!(plan["disk"]["path"], "/dev/sda");
    assert_eq!(plan["install_config"]["hostname"], "pre-host");
}

// r[verify installer.mode.interactive+2]
#[test]
fn dry_run_no_config_is_interactive() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);

    installer()
        .args([
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
    assert_eq!(plan["mode"], "interactive");
    assert_eq!(plan["disk_encryption"], "keyfile");
    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
}

// r[verify installer.dryrun]
// r[verify installer.config.copy-install-log]
#[test]
fn dry_run_manifest_path_is_null_when_no_images() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        hostname = "test-host"
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
    assert!(
        plan["manifest_path"].is_null(),
        "manifest_path should be null in dry-run without actual images"
    );
    assert!(
        plan["copy_install_log"].as_bool().unwrap(),
        "copy_install_log should default to true"
    );
}

// r[verify installer.config.format]
#[test]
fn multiple_validation_warnings_collected() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "none"
        disk = "largest-ssd"

        hostname = "-bad-"
        ssh-authorized-keys = [""]
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
        warnings.len() >= 2,
        "expected at least 2 warnings (bad hostname, empty ssh key), got: {warnings:?}"
    );
}
