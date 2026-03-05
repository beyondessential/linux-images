use super::common::{Fixture, SINGLE_SSD_DEVICE, installer};

// r[verify installer.dryrun.devices]
#[test]
fn error_no_devices_file() {
    let f = Fixture::new();
    let bad_path = f.path("nonexistent.json");

    installer()
        .args([
            "--fake-devices",
            bad_path.to_str().unwrap(),
            "--dry-run",
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("fake devices"));
}

#[test]
fn error_empty_devices_list() {
    let f = Fixture::new();
    let devices = f.write_devices("[]");

    installer()
        .args([
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("no block devices"));
}

// r[verify installer.dryrun.devices]
#[test]
fn error_invalid_devices_json() {
    let f = Fixture::new();
    let devices = f.write("devices.json", "this is not json");

    installer()
        .args([
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("fake devices"));
}

// r[verify installer.config.schema+3]
#[test]
fn error_invalid_config_toml() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config("this is not valid toml {{{{");

    installer()
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
        .failure()
        .stderr(predicates::str::contains("parsing config"));
}

// r[verify installer.config.schema+3]
#[test]
fn error_unknown_config_field() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config("bogus = true\n");

    installer()
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
        .failure()
        .stderr(predicates::str::contains("parsing config"));
}

// r[verify installer.config.schema+3]
#[test]
fn error_invalid_variant_in_config() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(r#"variant = "turbo""#);

    installer()
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
        .failure();
}

// r[verify installer.dryrun.script]
#[test]
fn error_bad_script_token() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let script = f.write_script("enter\nfoobar\n");

    installer()
        .args([
            "--fake-devices",
            devices.to_str().unwrap(),
            "--input-script",
            script.to_str().unwrap(),
            "--dry-run",
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("foobar"));
}

// r[verify installer.dryrun.script]
#[test]
fn error_nonexistent_script_file() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let bad_path = f.path("nonexistent.script");

    installer()
        .args([
            "--fake-devices",
            devices.to_str().unwrap(),
            "--input-script",
            bad_path.to_str().unwrap(),
            "--dry-run",
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("script"));
}

#[test]
fn error_disk_path_not_found() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "/dev/nonexistent"

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
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("not found"));
}

#[test]
fn error_no_ssds_for_largest_ssd_strategy() {
    let f = Fixture::new();
    // Only SATA disks, no SSDs
    let devices = f.write_devices(
        r#"[{"path": "/dev/sda", "size_bytes": 1000000000000, "model": "HDD", "transport": "Sata"}]"#,
    );
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
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
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("SSD"));
}
