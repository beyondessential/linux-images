use super::common::{Fixture, SINGLE_SSD_DEVICE, TWO_DISK_DEVICES, installer};

// r[verify installer.mode.prefilled]
// r[verify installer.dryrun.script]
#[test]
fn prefilled_accepting_defaults_produces_matching_plan() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        variant = "metal"
        disk = "/dev/nvme0n1"

        [firstboot]
        hostname = "prefilled-host"
        tailscale-authkey = "tskey-auth-123"
        ssh-authorized-keys = ["ssh-ed25519 AAAA key1"]
    "#,
    );
    // Walk through accepting all defaults: welcome, disk, variant, tpm, hostname,
    // tailscale, ssh, confirm
    let script = f.write_script(
        "\
# Welcome
enter
# Disk
enter
# Variant
enter
# TpmToggle
enter
# Hostname
enter
# Tailscale
enter
# SshKeys: Tab -> GitHub, Tab -> advance
tab
tab
# Password: skip (empty)
enter
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
    assert_eq!(plan["variant"], "metal");
    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
    assert_eq!(plan["firstboot"]["hostname"], "prefilled-host");
    assert!(plan["firstboot"]["tailscale_authkey"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["ssh_authorized_keys_count"], 1);
    assert!(!plan["firstboot"]["password_set"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["timezone"], "UTC");
}

// r[verify installer.mode.prefilled]
// r[verify installer.dryrun.script]
#[test]
fn prefilled_overriding_values_via_tui() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    let config = f.write_config(
        r#"
        variant = "metal"
        disk = "/dev/nvme0n1"

        [firstboot]
        hostname = "old-host"
    "#,
    );
    // Walk through: welcome, select second disk, switch to cloud, set new
    // hostname (clear old, type new), skip tailscale, skip ssh, confirm
    let script = f.write_script(
        "\
# Welcome
enter
# Disk: move down to second, accept
down
enter
# Variant: toggle to cloud
down
enter
# Hostname: clear 'old-host' (8 chars), type 'new-host'
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
# Tailscale
enter
# SSH keys: Tab -> GitHub, Tab -> advance
tab
tab
# Password: skip (empty)
enter
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
    assert_eq!(plan["variant"], "cloud");
    assert_eq!(plan["disk"]["path"], "/dev/sda");
    assert_eq!(plan["firstboot"]["hostname"], "new-host");
}

// r[verify installer.tui.timezone]
#[test]
fn prefilled_timezone_from_config() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let timezones = f.write_timezones();
    let config = f.write_config(
        r#"
        variant = "cloud"
        disk = "/dev/nvme0n1"

        [firstboot]
        timezone = "Europe/London"
    "#,
    );
    // Accept all defaults including the prefilled timezone
    let script = f.write_script(
        "\
# Welcome
enter
# Disk
enter
# Variant
enter
# Hostname
enter
# Tailscale
enter
# SshKeys: Tab -> GitHub, Tab -> advance
tab
tab
# Password: skip
enter
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
    assert_eq!(plan["firstboot"]["timezone"], "Europe/London");
}
