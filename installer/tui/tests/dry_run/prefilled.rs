use super::common::{Fixture, SINGLE_SSD_DEVICE, TWO_DISK_DEVICES, installer};

// r[verify installer.mode.prefilled]
// r[verify installer.dryrun.script]
#[test]
fn prefilled_accepting_defaults_produces_matching_plan() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        disk-encryption = "tpm"
        disk = "/dev/nvme0n1"

        hostname = "prefilled-host"
        tailscale-authkey = "tskey-auth-123"
        ssh-authorized-keys = ["ssh-ed25519 AAAA key1"]
    "#,
    );
    // Walk through accepting all defaults: welcome, disk, disk-encryption, hostname,
    // tailscale, ssh, confirm
    let script = f.write_script(
        "\
# Welcome
enter
# NetworkConfig: ISO -> Target
enter
# NetworkConfig: Target -> DiskSelection
enter
# Disk: accept default
enter
# DiskEncryption: accept default (tpm)
enter
# Hostname selector: Static is default (hostname prefilled from config), Enter -> HostnameInput
enter
# HostnameInput: accept prefilled hostname, Enter -> Login
enter
# Login: type password
type:pw
enter
type:pw
enter
# Timezone: accept default (UTC)
enter
# NetworkResults
enter
type:yes
enter
",
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
        .success();

    let plan = f.read_plan();
    assert_eq!(plan["mode"], "prefilled");
    assert_eq!(plan["disk_encryption"], "tpm");
    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
    assert_eq!(plan["install_config"]["hostname"], "prefilled-host");
    assert!(
        plan["install_config"]["tailscale_authkey"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(plan["install_config"]["ssh_authorized_keys_count"], 1);
    assert!(plan["install_config"]["password_set"].as_bool().unwrap());
    assert_eq!(plan["install_config"]["timezone"], "UTC");
}

// r[verify installer.mode.prefilled]
// r[verify installer.dryrun.script]
#[test]
fn prefilled_overriding_values_via_tui() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    let config = f.write_config(
        r#"
        disk-encryption = "tpm"
        disk = "/dev/nvme0n1"

        hostname = "old-host"
    "#,
    );
    // Walk through: welcome, select second disk, cycle encryption to none, set new
    // hostname (clear old, type new), skip tailscale, skip ssh, confirm
    let script = f.write_script(
        "\
# Welcome
enter
# NetworkConfig: ISO -> Target
enter
# NetworkConfig: Target -> DiskSelection
enter
# Disk: move down to second, accept (extra enter was already present via 'down' + 'enter')
down
enter
# DiskEncryption: cycle Tpm -> Keyfile -> None
down
down
enter
# Hostname selector: Static is default (hostname prefilled from config), Enter -> HostnameInput
enter
# HostnameInput: clear 'old-host' (8 chars), type 'new-host'
backspace
backspace
backspace
backspace
backspace
backspace
backspace
backspace
type:new-host
enter
# Login: type password
type:pw
enter
type:pw
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
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
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
    assert_eq!(plan["mode"], "prefilled");
    assert_eq!(plan["disk_encryption"], "none");
    assert_eq!(plan["disk"]["path"], "/dev/sda");
    assert_eq!(plan["install_config"]["hostname"], "new-host");
}

// r[verify installer.tui.timezone]
#[test]
fn prefilled_timezone_from_config() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let timezones = f.write_timezones();
    let config = f.write_config(
        r#"
        disk-encryption = "none"
        disk = "/dev/nvme0n1"

        timezone = "Europe/London"
    "#,
    );
    // Accept all defaults including the prefilled timezone
    let script = f.write_script(
        "\
# Welcome
enter
# NetworkConfig: ISO -> Target
enter
# NetworkConfig: Target -> DiskSelection
enter
# Disk: accept default
enter
# DiskEncryption: accept default (none, from config)
enter
# Hostname selector: Network-assigned is default, Enter -> Login
enter
# Login: type password
type:pw
enter
type:pw
enter
# Timezone: accept prefilled Europe/London
enter
# NetworkResults
enter
type:yes
enter
",
    );

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
        .success();

    let plan = f.read_plan();
    assert_eq!(plan["install_config"]["timezone"], "Europe/London");
}
