use super::common::{Fixture, SINGLE_SSD_DEVICE, installer};

// r[verify installer.tui.timezone]
// r[verify installer.firstboot.timezone]
// r[verify installer.dryrun.schema+3]
#[test]
fn auto_timezone_defaults_to_utc() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "none"
        disk = "largest-ssd"

        [firstboot]
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
    assert_eq!(plan["firstboot"]["timezone"], "UTC");
}

// r[verify installer.tui.timezone]
// r[verify installer.firstboot.timezone]
// r[verify installer.config.schema+3]
#[test]
fn auto_timezone_from_config() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "none"
        disk = "largest-ssd"

        [firstboot]
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
    assert_eq!(plan["firstboot"]["timezone"], "Pacific/Auckland");
}

// r[verify installer.tui.timezone]
// r[verify installer.dryrun.schema+3]
#[test]
fn auto_metal_timezone_from_config() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk-encryption = "tpm"
        disk = "largest-ssd"

        [firstboot]
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
    assert_eq!(plan["firstboot"]["hostname"], "tz-test");
    assert_eq!(plan["firstboot"]["timezone"], "America/New_York");
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
# DiskSelection
enter
# DiskEncryptionScreen: cycle to None (cloud)
down
enter
# Hostname selector: network-assigned is default for cloud, Enter -> Login
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
    assert_eq!(plan["firstboot"]["timezone"], "Pacific/Auckland");
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
# DiskSelection
enter
# DiskEncryptionScreen: cycle to None (cloud)
down
enter
# Hostname selector: network-assigned is default for cloud, Enter -> Login
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
    assert_eq!(plan["firstboot"]["timezone"], "Europe/London");
}
