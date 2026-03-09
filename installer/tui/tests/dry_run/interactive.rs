use super::common::{Fixture, SINGLE_SSD_DEVICE, TWO_DISK_DEVICES};

// r[verify installer.mode.interactive+2]
// r[verify installer.dryrun.script.headless]
#[test]
fn interactive_keyfile_full_flow() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(TWO_DISK_DEVICES)
        .timezones()
        .script(
            "\
# Welcome
enter
# NetworkConfig: ISO -> Target
enter
# NetworkConfig: Target -> DiskSelection
enter
# Disk: accept first
enter
# DiskEncryption: accept default (keyfile)
enter
# Hostname selector: DHCP is default, toggle to Static
down
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
        )
        .run()
        .read_plan();

    assert_eq!(plan["mode"], "interactive");
    assert_eq!(plan["disk_encryption"], "keyfile");
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
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .timezones()
        .start_screen("disk-encryption")
        .script(
            "\
# DiskEncryption: cycle Keyfile -> None
down
enter
# Hostname selector: Network-assigned is default, Enter -> Login
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
        )
        .run()
        .read_plan();

    assert_eq!(plan["disk_encryption"], "none");
}

// r[verify installer.tui.hostname+6]
// r[verify installer.tui.tailscale+3]
// r[verify installer.tui.ssh-keys+5]
#[test]
fn interactive_install_config_fields_captured() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .timezones()
        .start_screen("hostname")
        .script(
            "\
# Hostname selector: DHCP is default, toggle to Static
down
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
        )
        .run()
        .read_plan();

    assert_eq!(plan["install_config"]["hostname"], "my-host");
    assert!(
        plan["install_config"]["tailscale_authkey"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(plan["install_config"]["ssh_authorized_keys_count"], 2);
    assert_eq!(plan["install_config"]["timezone"], "UTC");
}

// r[verify installer.tui.hostname+6]
// r[verify installer.tui.tailscale+3]
// r[verify installer.tui.ssh-keys+5]
// r[verify installer.tui.password+4]
#[test]
fn interactive_empty_install_config_is_null() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .timezones()
        .start_screen("login")
        .script(
            "\
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
        )
        .run()
        .read_plan();

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

// r[verify installer.tui.disk-detection+4]
#[test]
fn interactive_selects_second_disk() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(TWO_DISK_DEVICES)
        .timezones()
        .start_screen("disk-selection")
        .script(
            "\
# Navigate to second disk
down
enter
# DiskEncryption: accept default (keyfile)
enter
# Hostname selector: DHCP is default, toggle to Static
down
enter
# HostnameInput: type 'h'
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
        )
        .run()
        .read_plan();

    assert_eq!(plan["disk"]["path"], "/dev/sda");
    assert_eq!(plan["disk"]["model"], "WD Blue");
}

// r[verify installer.dryrun.script.headless]
#[test]
fn interactive_quit_early_still_produces_plan() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .script("type:q\n")
        .run()
        .read_plan();

    assert_eq!(plan["mode"], "interactive");
    assert_eq!(plan["disk_encryption"], "keyfile");
    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
}

// r[verify installer.dryrun.script.headless]
#[test]
fn interactive_empty_script_uses_initial_state() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .script("")
        .run()
        .read_plan();

    assert_eq!(plan["mode"], "interactive");
    assert_eq!(plan["disk_encryption"], "keyfile");
}

// r[verify installer.tui.confirmation+8]
#[test]
fn interactive_go_back_from_confirmation_and_change() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .timezones()
        .start_screen("login")
        .script(
            "\
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
        )
        .run()
        .read_plan();

    assert!(
        plan["install_config"]["tailscale_authkey"]
            .as_bool()
            .unwrap()
    );
}
