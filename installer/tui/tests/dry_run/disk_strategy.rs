use super::common::{Fixture, THREE_MIXED_DEVICES, installer};

// r[verify installer.config.disk]
#[test]
fn strategy_largest_ssd_picks_biggest_nvme() {
    let f = Fixture::new();
    let devices = f.write_devices(THREE_MIXED_DEVICES);
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
    assert_eq!(plan["disk"]["path"], "/dev/nvme1n1");
    assert_eq!(plan["disk"]["model"], "Big NVMe");
}

// r[verify installer.config.disk]
#[test]
fn strategy_largest_picks_biggest_overall() {
    let f = Fixture::new();
    let devices = f.write_devices(THREE_MIXED_DEVICES);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest"

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
    assert_eq!(plan["disk"]["path"], "/dev/sda");
    assert_eq!(plan["disk"]["size_bytes"], 2000000000000u64);
}

// r[verify installer.config.disk]
#[test]
fn strategy_smallest_picks_smallest_overall() {
    let f = Fixture::new();
    let devices = f.write_devices(THREE_MIXED_DEVICES);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "smallest"

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
    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
    assert_eq!(plan["disk"]["size_bytes"], 500000000000u64);
}

// r[verify installer.config.disk]
#[test]
fn strategy_disk_path_selects_exact_device() {
    let f = Fixture::new();
    let devices = f.write_devices(THREE_MIXED_DEVICES);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "none"
        disk = "/dev/nvme1n1"
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
    assert_eq!(plan["disk"]["path"], "/dev/nvme1n1");
    assert_eq!(plan["disk"]["model"], "Big NVMe");
}
