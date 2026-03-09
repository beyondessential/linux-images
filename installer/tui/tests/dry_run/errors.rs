use super::common::{Fixture, SINGLE_SSD_DEVICE};

// r[verify installer.dryrun.devices]
#[test]
fn error_no_devices_file() {
    let f = Fixture::new();
    let bad_path = f.path("nonexistent.json");

    super::common::installer()
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

    f.scripted_run("[]")
        .build()
        .assert()
        .failure()
        .stderr(predicates::str::contains("no block devices"));
}

// r[verify installer.dryrun.devices]
#[test]
fn error_invalid_devices_json() {
    let f = Fixture::new();
    let devices = f.write("devices.json", "this is not json");

    super::common::installer()
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

// r[verify installer.config.format]
#[test]
fn error_invalid_config_toml() {
    let f = Fixture::new();

    f.scripted_run(SINGLE_SSD_DEVICE)
        .config("this is not valid toml {{{{")
        .build()
        .assert()
        .failure()
        .stderr(predicates::str::contains("parsing config"));
}

// r[verify installer.config.format]
#[test]
fn error_unknown_config_field() {
    let f = Fixture::new();

    f.scripted_run(SINGLE_SSD_DEVICE)
        .config("bogus = true\n")
        .build()
        .assert()
        .failure()
        .stderr(predicates::str::contains("parsing config"));
}

// r[verify installer.config.format]
#[test]
fn error_invalid_variant_in_config() {
    let f = Fixture::new();

    f.scripted_run(SINGLE_SSD_DEVICE)
        .config(r#"variant = "turbo""#)
        .build()
        .assert()
        .failure();
}

// r[verify installer.dryrun.script]
#[test]
fn error_bad_script_token() {
    let f = Fixture::new();

    f.scripted_run(SINGLE_SSD_DEVICE)
        .script("enter\nfoobar\n")
        .build()
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

    super::common::installer()
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

    f.scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            auto = true
            disk-encryption = "tpm"
            disk = "/dev/nonexistent"

            hostname = "test-host"
        "#,
        )
        .build()
        .assert()
        .failure()
        .stderr(predicates::str::contains("not found"));
}

#[test]
fn error_logged_to_file() {
    let f = Fixture::new();
    // Trigger a guaranteed error: no devices file at this path
    let bad_path = f.path("nonexistent.json");

    super::common::installer()
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

    // The same error must also appear in the log file
    let log_contents = std::fs::read_to_string(f.log_path()).unwrap();
    assert!(
        log_contents.contains("fake devices"),
        "expected log file to contain the error, got: {log_contents}"
    );
}

#[test]
fn error_no_ssds_for_largest_ssd_strategy() {
    let f = Fixture::new();

    f.scripted_run(
        r#"[{"path": "/dev/sda", "size_bytes": 1000000000000, "model": "HDD", "transport": "Sata"}]"#,
    )
    .config(
        r#"
            auto = true
            disk-encryption = "tpm"
            disk = "largest-ssd"

            hostname = "test-host"
        "#,
    )
    .build()
    .assert()
    .failure()
    .stderr(predicates::str::contains("SSD"));
}
