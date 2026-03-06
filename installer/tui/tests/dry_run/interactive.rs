use super::common::{Fixture, SINGLE_SSD_DEVICE, TWO_DISK_DEVICES, installer};

// r[verify installer.mode.interactive+2]
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
# DiskEncryption: accept default (keyfile)
enter
# Hostname selector: Static is default for encrypted, Enter -> HostnameInput
enter
# HostnameInput: type hostname then advance
type:my-server
enter
# Login: enter tailscale sub-screen
alt:t
type:tskey-auth-test
enter
# Login: enter ssh keys sub-screen
alt:s
type:ssh-ed25519 AAAA testkey
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
    assert_eq!(plan["disk_encryption"], "keyfile");
    assert_eq!(plan["variant"], "metal");
    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
    assert_eq!(plan["install_config"]["hostname"], "my-server");
    assert!(
        plan["install_config"]["tailscale_authkey"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(plan["install_config"]["ssh_authorized_keys_count"], 1);
    assert_eq!(plan["install_config"]["timezone"], "UTC");
}

// r[verify installer.mode.interactive+2]
// r[verify installer.tui.disk-encryption+2]
#[test]
fn interactive_none_encryption_flow() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    // None encryption flow: welcome, disk, cycle encryption Keyfile->None,
    // hostname, login, confirm. Single DiskEncryption screen.
    let script = f.write_script(
        "\
# Welcome
enter
# Disk
enter
# DiskEncryption: cycle Keyfile -> None
down
enter
# Hostname selector: network-assigned is default for none, Enter -> Login
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
    assert_eq!(plan["disk_encryption"], "none");
    assert_eq!(plan["variant"], "cloud");
}

// r[verify installer.tui.hostname+5]
// r[verify installer.tui.tailscale+3]
// r[verify installer.tui.ssh-keys+5]
#[test]
fn interactive_install_config_fields_captured() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let script = f.write_script(
        "\
# Welcome
enter
# Disk
enter
# DiskEncryption: cycle Keyfile -> None
down
enter
# Hostname selector: network-assigned is default for none, Up to select Static
up
enter
# HostnameInput: type hostname then advance
type:my-host
enter
# Login: enter tailscale sub-screen
alt:t
type:tskey-auth-mykey
enter
# Login: enter ssh keys sub-screen
alt:s
type:ssh-ed25519 AAAA key1
tab
type:ssh-rsa BBBB key2
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
    assert_eq!(plan["install_config"]["hostname"], "my-host");
    assert!(
        plan["install_config"]["tailscale_authkey"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(plan["install_config"]["ssh_authorized_keys_count"], 2);
    assert_eq!(plan["install_config"]["timezone"], "UTC");
}

// r[verify installer.tui.hostname+5]
// r[verify installer.tui.tailscale+3]
// r[verify installer.tui.ssh-keys+5]
// r[verify installer.tui.password+4]
#[test]
fn interactive_empty_install_config_is_null() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    // Use none encryption — select network-assigned (the default for none) to
    // skip hostname entirely. Password is the only required install config field
    // in interactive mode.
    let script = f.write_script(
        "\
# Welcome
enter
# Disk
enter
# DiskEncryption: cycle Keyfile -> None
down
enter
# Hostname selector: network-assigned is default for none, Enter -> Login
enter
# Login: type password (required)
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
    // Password is always set in interactive mode, so install_config is not null
    assert!(!plan["install_config"].is_null());
    assert!(plan["install_config"]["password_set"].as_bool().unwrap());
    // Hostname is "dhcp" sentinel when network-assigned is selected
    assert_eq!(
        plan["install_config"]["hostname"], "dhcp",
        "hostname should be dhcp sentinel when network-assigned is selected"
    );
    assert!(
        !plan["install_config"]["tailscale_authkey"]
            .as_bool()
            .unwrap(),
        "tailscale should be false when skipped"
    );
    assert_eq!(
        plan["install_config"]["ssh_authorized_keys_count"], 0,
        "ssh keys should be empty when skipped"
    );
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
# DiskEncryption: accept default (keyfile)
enter
# Hostname selector: Static is default for encrypted, Enter -> HostnameInput
enter
# HostnameInput: type 'h' (required for encrypted)
type:h
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
    assert_eq!(plan["disk_encryption"], "keyfile");
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
    assert_eq!(plan["disk_encryption"], "keyfile");
    assert_eq!(plan["variant"], "metal");
}

// r[verify installer.tui.confirmation+6]
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
# DiskEncryption: cycle Keyfile -> None
down
enter
# Hostname selector: network-assigned is default for none, Enter -> Login
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
# Now on Confirmation, go back
esc
# Back on NetworkResults, go back
esc
# Back on Timezone, go back
esc
# Back on Login, enter tailscale sub-screen
alt:t
type:tskey-auth-late
enter
# Login: advance (password already set, confirming=false so just advance)
type:pw
enter
type:pw
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
    assert!(
        plan["install_config"]["tailscale_authkey"]
            .as_bool()
            .unwrap()
    );
}
