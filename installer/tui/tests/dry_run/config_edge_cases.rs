use super::common::{Fixture, SINGLE_SSD_DEVICE, installer};

// r[verify installer.dryrun.devices]
#[test]
fn fake_devices_removable_field_optional() {
    let f = Fixture::new();
    let devices = f.write_devices(
        r#"[{"path": "/dev/sda", "size_bytes": 100000000000, "model": "Test", "transport": "Nvme"}]"#,
    );
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
        disk = "largest-ssd"

        [firstboot]
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
}

// r[verify installer.dryrun.devices]
#[test]
fn fake_devices_transport_aliases_accepted() {
    let f = Fixture::new();
    // Use display-form aliases: "NVMe", "SATA", etc.
    let devices = f.write_devices(
        r#"[
            {"path": "/dev/nvme0n1", "size_bytes": 100000000000, "model": "A", "transport": "NVMe"},
            {"path": "/dev/sda", "size_bytes": 200000000000, "model": "B", "transport": "SATA"}
        ]"#,
    );
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
        disk = "largest"

        [firstboot]
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
}

// r[verify installer.config.schema+2]
#[test]
fn empty_config_file_treated_as_prefilled() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config("");

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
    // Empty config has auto=false by default -> prefilled mode
    assert_eq!(plan["mode"], "prefilled");
}
