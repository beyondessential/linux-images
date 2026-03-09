use super::common::{Fixture, SINGLE_SSD_DEVICE, TWO_DISK_DEVICES};

// r[verify installer.mode.prefilled]
// r[verify installer.dryrun.script]
#[test]
fn prefilled_accepting_defaults_produces_matching_plan() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            disk-encryption = "tpm"
            disk = "/dev/nvme0n1"

            hostname = "prefilled-host"
            tailscale-authkey = "tskey-auth-123"
            ssh-authorized-keys = ["ssh-ed25519 AAAA key1"]
        "#,
        )
        .timezones()
        .script(
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
        )
        .run()
        .read_plan();

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
    let plan = f
        .scripted_run(TWO_DISK_DEVICES)
        .config(
            r#"
            disk-encryption = "tpm"
            disk = "/dev/nvme0n1"

            hostname = "old-host"
        "#,
        )
        .start_screen("disk-selection")
        .script(
            "\
# Disk: move down to second, accept
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
        )
        .run()
        .read_plan();

    assert_eq!(plan["mode"], "prefilled");
    assert_eq!(plan["disk_encryption"], "none");
    assert_eq!(plan["disk"]["path"], "/dev/sda");
    assert_eq!(plan["install_config"]["hostname"], "new-host");
}

// r[verify installer.tui.timezone]
#[test]
fn prefilled_timezone_from_config() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            disk-encryption = "none"
            disk = "/dev/nvme0n1"

            timezone = "Europe/London"
        "#,
        )
        .timezones()
        .start_screen("hostname")
        .script(
            "\
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
        )
        .run()
        .read_plan();

    assert_eq!(plan["install_config"]["timezone"], "Europe/London");
}
