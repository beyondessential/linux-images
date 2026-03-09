use super::common::{Fixture, SINGLE_SSD_DEVICE, TWO_DISK_DEVICES};

// r[verify installer.tui.disk-encryption+2]
#[test]
fn scripted_encryption_cycle_twice_returns_to_default() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .start_screen("disk-encryption")
        .script(
            "\
# DiskEncryptionScreen: default Keyfile, cycle down twice (Keyfile->None->Keyfile)
down
down
enter
# Hostname selector: Network-assigned is default, Up to select Static
up
enter
# HostnameInput: type 'h'
type:h
enter
# Login: type password
type:pw
enter
type:pw
enter
# Timezone: accept default
enter
# NetworkResults
enter
type:yes
enter
",
        )
        .run()
        .read_plan();

    assert!(!plan["tpm_present"].as_bool().unwrap());
    assert_eq!(plan["disk_encryption"], "keyfile");
}

// r[verify installer.tui.disk-encryption+2]
#[test]
fn scripted_encryption_cycle_back_to_keyfile() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .start_screen("disk-encryption")
        .script(
            "\
# DiskEncryptionScreen: default Keyfile, cycle: Keyfile->None->Keyfile
down
down
enter
# Hostname selector: Network-assigned is default, Up to select Static
up
enter
# HostnameInput: type 'h'
type:h
enter
# Login: type password
type:pw
enter
type:pw
enter
# Timezone: accept default
enter
# NetworkResults
enter
type:yes
enter
",
        )
        .run()
        .read_plan();

    assert_eq!(plan["disk_encryption"], "keyfile");
}

// r[verify installer.tui.disk-detection+4]
#[test]
fn scripted_disk_wrap_around() {
    let f = Fixture::new();
    // Two devices: index 0 = nvme0n1, index 1 = sda.
    // Navigate down twice to wrap around back to index 0.
    let plan = f
        .scripted_run(TWO_DISK_DEVICES)
        .start_screen("disk-selection")
        .script(
            "\
# Disk: down twice wraps back to index 0
down
down
enter
# DiskEncryptionScreen: accept default (Keyfile)
enter
# Hostname selector: Network-assigned is default, Up to select Static
up
enter
# HostnameInput: type 'h'
type:h
enter
# Login: type password
type:pw
enter
type:pw
enter
# Timezone: accept default
enter
# NetworkResults
enter
type:yes
enter
",
        )
        .run()
        .read_plan();

    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
}

// r[verify installer.tui.disk-detection+4]
#[test]
fn scripted_disk_up_wraps_to_last() {
    let f = Fixture::new();
    // Starting at index 0, up wraps to index 1 (last device)
    let plan = f
        .scripted_run(TWO_DISK_DEVICES)
        .start_screen("disk-selection")
        .script(
            "\
# Disk: up wraps to last
up
enter
# DiskEncryptionScreen: accept default (Keyfile)
enter
# Hostname selector: Network-assigned is default, Up to select Static
up
enter
# HostnameInput: type 'h'
type:h
enter
# Login: type password
type:pw
enter
type:pw
enter
# Timezone: accept default
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
}

// r[verify installer.tui.hostname+6]
#[test]
fn scripted_hostname_with_backspace_correction() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .start_screen("hostname")
        .script(
            "\
# Hostname selector: Network-assigned is default, Up to select Static
up
enter
# HostnameInput: type 'baaad', backspace 3 times, type 'd'
type:baaad
backspace
backspace
backspace
type:d
enter
# Login: type password
type:pw
enter
type:pw
enter
# Timezone: accept default
enter
# NetworkResults
enter
type:yes
enter
",
        )
        .run()
        .read_plan();

    assert_eq!(plan["install_config"]["hostname"], "bad");
}

// r[verify installer.tui.ssh-keys+5]
#[test]
fn scripted_multiline_ssh_keys() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .start_screen("login")
        .script(
            "\
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
# Timezone: accept default
enter
# NetworkResults
enter
type:yes
enter
",
        )
        .run()
        .read_plan();

    assert_eq!(plan["install_config"]["ssh_authorized_keys_count"], 2);
}

// r[verify installer.tui.confirmation+8]
#[test]
fn scripted_wrong_confirmation_does_not_advance() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .start_screen("confirmation")
        .script(
            "\
type:no
enter
",
        )
        .run()
        .read_plan();

    // Plan is still produced (from current state at script end)
    assert_eq!(plan["mode"], "interactive");
}
