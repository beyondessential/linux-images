use super::common::{Fixture, SINGLE_SSD_DEVICE, TWO_DISK_DEVICES, installer};

// r[verify installer.mode.interactive]
// r[verify installer.dryrun.script.headless]
#[test]
fn interactive_metal_full_flow() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    let script = f.write_script(
        "\
# Welcome
enter
# Disk: accept first
enter
# Variant: accept metal default
enter
# TpmToggle: disable, advance
space
enter
# Hostname
type:my-server
enter
# Login: enter tailscale sub-screen
type:t
type:tskey-auth-test
enter
# Login: enter ssh keys sub-screen
type:s
type:ssh-ed25519 AAAA testkey
enter
# Login: skip password (empty)
enter
enter
# Timezone: accept default (UTC)
enter
# NetworkResults
enter
# Confirm
type:yes
enter
",
    );
    let timezones = f.write_timezones();

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
    assert_eq!(plan["mode"], "interactive");
    assert_eq!(plan["variant"], "metal");
    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
    assert!(plan["disable_tpm"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["hostname"], "my-server");
    assert!(plan["firstboot"]["tailscale_authkey"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["ssh_authorized_keys_count"], 1);
    assert_eq!(plan["firstboot"]["timezone"], "UTC");
}

// r[verify installer.mode.interactive]
// r[verify installer.tui.variant-selection]
// r[verify image.tpm.disableable]
#[test]
fn interactive_cloud_skips_tpm_screen() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    // Cloud flow: welcome, disk, toggle variant to cloud, hostname, tailscale,
    // ssh, confirm. No TpmToggle screen.
    let script = f.write_script(
        "\
# Welcome
enter
# Disk
enter
# Variant: toggle to cloud
down
enter
# Hostname: skip
enter
# Login: skip password (empty)
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
    assert_eq!(plan["variant"], "cloud");
    assert!(!plan["disable_tpm"].as_bool().unwrap());
}

// r[verify installer.tui.hostname+2]
// r[verify installer.tui.tailscale+2]
// r[verify installer.tui.ssh-keys+2]
#[test]
fn interactive_firstboot_fields_captured() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let script = f.write_script(
        "\
# Welcome
enter
# Disk
enter
# Variant: toggle to cloud
down
enter
type:my-host
enter
# Login: enter tailscale sub-screen
type:t
type:tskey-auth-mykey
enter
# Login: enter ssh keys sub-screen
type:s
type:ssh-ed25519 AAAA key1
tab
type:ssh-rsa BBBB key2
enter
# Login: skip password (empty)
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
    assert_eq!(plan["firstboot"]["hostname"], "my-host");
    assert!(plan["firstboot"]["tailscale_authkey"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["ssh_authorized_keys_count"], 2);
    assert_eq!(plan["firstboot"]["timezone"], "UTC");
}

// r[verify installer.tui.hostname+2]
// r[verify installer.tui.tailscale+2]
// r[verify installer.tui.ssh-keys+2]
#[test]
fn interactive_empty_firstboot_is_null() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    // Use cloud variant so hostname is optional — all empty firstboot fields yield null
    let script = f.write_script(
        "\
# Welcome
enter
# Disk
enter
# Toggle to cloud (hostname optional)
down
enter
# Hostname: skip
enter
# Login: skip password (empty)
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
    assert!(plan["firstboot"].is_null());
}

// r[verify installer.tui.disk-detection+3]
#[test]
fn interactive_selects_second_disk() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    let script = f.write_script(
        "\
# Welcome
enter
# Navigate to second disk
down
enter
# Variant
enter
# TpmToggle
enter
# Hostname: type 'h' (required for metal)
type:h
enter
# Login: skip password (empty)
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
    assert_eq!(plan["disk"]["path"], "/dev/sda");
    assert_eq!(plan["disk"]["model"], "WD Blue");
}

// r[verify installer.dryrun.script.headless]
#[test]
fn interactive_quit_early_still_produces_plan() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    // Quit immediately on the welcome screen
    let script = f.write_script("type:q\n");

    installer()
        .args([
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
    assert_eq!(plan["mode"], "interactive");
    assert_eq!(plan["variant"], "metal");
    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
}

// r[verify installer.dryrun.script.headless]
#[test]
fn interactive_empty_script_uses_initial_state() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let script = f.write_script("");

    installer()
        .args([
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
    assert_eq!(plan["mode"], "interactive");
    assert_eq!(plan["variant"], "metal");
}

// r[verify installer.tui.confirmation+3]
#[test]
fn interactive_go_back_from_confirmation_and_change() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    // Walk to confirmation, go back through timezone/password to tailscale,
    // type a key, then walk forward again and confirm.
    let script = f.write_script(
        "\
# Welcome
enter
# Disk
enter
# Variant: toggle to cloud
down
enter
# Hostname: skip
enter
# Login: skip password (empty)
enter
enter
# Timezone: accept default (UTC)
enter
# NetworkResults
enter
# Now on Confirmation, go back
esc
# Back on NetworkResults, go back
esc
# Back on Timezone, go back
esc
# Back on Login, enter tailscale sub-screen
type:t
type:tskey-auth-late
enter
# Login: skip password (still empty)
enter
enter
# Timezone: accept default (UTC)
enter
# NetworkResults
enter
# Confirmation
type:yes
enter
",
    );
    let timezones = f.write_timezones();

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
    assert!(plan["firstboot"]["tailscale_authkey"].as_bool().unwrap());
}
