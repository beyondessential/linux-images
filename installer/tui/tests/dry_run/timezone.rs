use super::common::{Fixture, SINGLE_SSD_DEVICE, installer};

// r[verify installer.tui.timezone]
// r[verify installer.finalise.timezone]
// r[verify installer.dryrun.schema+6]
#[test]
fn auto_timezone_defaults_to_utc() {
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
    // When no install-time fields are set, install_config is null.
    // The effective timezone defaults to UTC, but there is no install_config
    // object to carry it.
    assert!(
        plan["install_config"].is_null(),
        "install_config should be null when no fields are configured"
    );
}

// r[verify installer.tui.timezone]
// r[verify installer.finalise.timezone]
// r[verify installer.config.timezone]
#[test]
fn auto_timezone_from_config() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "none"
        disk = "largest-ssd"

        timezone = "Pacific/Auckland"
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
    assert_eq!(plan["install_config"]["timezone"], "Pacific/Auckland");
}

// r[verify installer.tui.timezone]
// r[verify installer.dryrun.schema+6]
#[test]
fn auto_encrypted_timezone_from_config() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        hostname = "tz-test"
        timezone = "America/New_York"
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
    assert_eq!(plan["install_config"]["hostname"], "tz-test");
    assert_eq!(plan["install_config"]["timezone"], "America/New_York");
}

// r[verify installer.tui.timezone]
#[test]
fn scripted_timezone_search_and_select() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let timezones = f.write_timezones();
    let script = f.write_script(
        "\
# Welcome
enter
# NetworkConfig
enter
# DiskSelection
enter
# DiskEncryptionScreen: cycle to None
down
enter
# Hostname selector: Network-assigned is default, Enter -> Login
enter
# Login: type password
type:pw
enter
type:pw
enter
# Timezone: search for 'auck', select first match
type:auck
enter
# NetworkResults
enter
# Confirm
type:yes
enter
",
    );

    installer()
        .args([
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
        .success();

    let plan = f.read_plan();
    assert_eq!(plan["install_config"]["timezone"], "Pacific/Auckland");
}

// r[verify installer.tui.timezone]
#[test]
fn scripted_timezone_navigate_and_select() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let timezones = f.write_timezones();
    // Timezones in sorted order: America/New_York(0), Europe/London(1),
    // Pacific/Auckland(2), UTC(3). Default cursor at UTC (index 3).
    let script = f.write_script(
        "\
# Welcome
enter
# NetworkConfig
enter
# DiskSelection
enter
# DiskEncryptionScreen: cycle to None
down
enter
# Hostname selector: Network-assigned is default, Enter -> Login
enter
# Login: type password
type:pw
enter
type:pw
enter
# Timezone: up twice from UTC(3) -> Europe/London(1), then select
up
up
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
        .success();

    let plan = f.read_plan();
    assert_eq!(plan["install_config"]["timezone"], "Europe/London");
}
