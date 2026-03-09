use predicates::prelude::*;

use super::common::{Fixture, SINGLE_SSD_DEVICE, TWO_DISK_DEVICES};

// r[verify installer.dryrun]
// r[verify installer.dryrun.output]
// r[verify installer.dryrun.schema+6]
// r[verify installer.mode.auto+4]
#[test]
fn auto_full_config_produces_correct_plan() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(TWO_DISK_DEVICES)
        .config(
            r#"
            auto = true
            disk-encryption = "tpm"
            disk = "largest-ssd"

            hostname = "server-01"
            tailscale-authkey = "tskey-auth-xxxxx"
            ssh-authorized-keys = [
                "ssh-ed25519 AAAA admin@example.com",
                "ssh-rsa BBBB backup@example.com",
            ]
        "#,
        )
        .run()
        .read_plan();

    assert_eq!(plan["mode"], "auto");
    assert_eq!(plan["disk_encryption"], "tpm");
    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
    assert_eq!(plan["disk"]["model"], "Samsung 980 PRO");
    assert_eq!(plan["disk"]["size_bytes"], 1000204886016u64);
    assert_eq!(plan["disk"]["transport"], "NVMe");
    assert!(!plan["tpm_present"].as_bool().unwrap());
    assert_eq!(plan["install_config"]["hostname"], "server-01");
    assert!(
        plan["install_config"]["tailscale_authkey"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(plan["install_config"]["ssh_authorized_keys_count"], 2);
    assert!(plan["config_warnings"].as_array().unwrap().is_empty());
}

// r[verify installer.dryrun.schema+6]
// r[verify installer.config.disk]
#[test]
fn auto_disk_path_resolves_correctly() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(TWO_DISK_DEVICES)
        .config(
            r#"
            auto = true
            disk-encryption = "none"
            disk = "/dev/sda"
        "#,
        )
        .run()
        .read_plan();

    assert_eq!(plan["mode"], "auto");
    assert_eq!(plan["disk_encryption"], "none");
    assert_eq!(plan["disk"]["path"], "/dev/sda");
    assert_eq!(plan["disk"]["model"], "WD Blue");
}

// r[verify installer.dryrun.schema+6]
// r[verify installer.config.disk-encryption+2]
#[test]
fn auto_keyfile_encryption_mode() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            auto = true
            disk-encryption = "keyfile"
            disk = "largest-ssd"

            hostname = "test-keyfile"
        "#,
        )
        .run()
        .read_plan();

    assert_eq!(plan["disk_encryption"], "keyfile");
}

// r[verify installer.dryrun.schema+6]
// r[verify installer.dryrun.fake-tpm]
#[test]
fn auto_fake_tpm_reflected_in_plan() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            auto = true
            disk-encryption = "tpm"
            disk = "largest-ssd"

            hostname = "test-tpm"
        "#,
        )
        .fake_tpm()
        .run()
        .read_plan();

    assert!(plan["tpm_present"].as_bool().unwrap());
    assert_eq!(plan["disk_encryption"], "tpm");
}

// r[verify installer.dryrun.schema+6]
#[test]
fn auto_none_encryption_no_install_config() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            auto = true
            disk-encryption = "none"
            disk = "largest-ssd"
        "#,
        )
        .run()
        .read_plan();

    assert_eq!(plan["disk_encryption"], "none");
    assert!(plan["install_config"].is_null());
    assert!(!plan["tpm_present"].as_bool().unwrap());
}

// r[verify installer.config.hostname]
#[test]
fn auto_bad_hostname_emits_warning() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            auto = true
            disk-encryption = "none"
            disk = "largest-ssd"

            hostname = "-invalid-"
        "#,
        )
        .run()
        .read_plan();

    let warnings = plan["config_warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| w
            .as_str()
            .unwrap()
            .contains("must not start or end with a hyphen")),
        "expected a hostname validation warning, got: {warnings:?}"
    );
}

// r[verify installer.mode.auto-incomplete+3]
#[test]
fn auto_incomplete_missing_disk_encryption_falls_back() {
    let f = Fixture::new();
    f.scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            auto = true
            disk = "largest-ssd"
        "#,
        )
        .timezones()
        .script(
            "enter\nenter\nenter\nenter\ntype:h\nenter\nenter\ntab\nenter\nenter\nenter\ntype:yes\nenter\n",
        )
        .build()
        .assert()
        .success()
        .stderr(predicates::str::contains("disk-encryption"));

    let plan = f.read_plan();
    assert_eq!(plan["mode"], "auto-incomplete");
}

// r[verify installer.mode.auto-incomplete+3]
#[test]
fn auto_incomplete_missing_disk_falls_back() {
    let f = Fixture::new();
    f.scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            auto = true
            disk-encryption = "tpm"
        "#,
        )
        .timezones()
        .script(
            "enter\nenter\nenter\nenter\ntype:h\nenter\nenter\ntab\nenter\nenter\nenter\ntype:yes\nenter\n",
        )
        .build()
        .assert()
        .success()
        .stderr(predicates::str::contains("disk").and(predicates::str::contains("hostname")));

    let plan = f.read_plan();
    assert_eq!(plan["mode"], "auto-incomplete");
}

// r[verify installer.mode.auto-incomplete+3]
#[test]
fn auto_incomplete_missing_both_falls_back() {
    let f = Fixture::new();
    f.scripted_run(SINGLE_SSD_DEVICE)
        .config("auto = true\n")
        .timezones()
        .script(
            "enter\nenter\nenter\nenter\ntype:h\nenter\nenter\ntab\nenter\nenter\nenter\ntype:yes\nenter\n",
        )
        .build()
        .assert()
        .success()
        .stderr(
            predicates::str::contains("disk-encryption").and(predicates::str::contains("disk")),
        );

    let plan = f.read_plan();
    assert_eq!(plan["mode"], "auto-incomplete");
}

// r[verify installer.config.hostname]
#[test]
fn auto_with_minimal_install_config() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            auto = true
            disk-encryption = "none"
            disk = "largest-ssd"

            hostname = "just-a-hostname"
        "#,
        )
        .run()
        .read_plan();

    assert_eq!(plan["install_config"]["hostname"], "just-a-hostname");
    assert!(
        !plan["install_config"]["tailscale_authkey"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(plan["install_config"]["ssh_authorized_keys_count"], 0);
}

// r[verify installer.config.ssh-authorized-keys+2]
#[test]
fn auto_with_only_ssh_keys() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            auto = true
            disk-encryption = "tpm"
            disk = "largest-ssd"

            hostname = "test-host"
            ssh-authorized-keys = [
                "ssh-ed25519 AAAA k1",
                "ssh-ed25519 BBBB k2",
                "ssh-rsa CCCC k3",
            ]
        "#,
        )
        .run()
        .read_plan();

    assert_eq!(plan["install_config"]["hostname"], "test-host");
    assert!(
        !plan["install_config"]["tailscale_authkey"]
            .as_bool()
            .unwrap()
    );
    assert_eq!(plan["install_config"]["ssh_authorized_keys_count"], 3);
}

// r[verify installer.finalise.network+4]
// r[verify installer.dryrun.schema+6]
#[test]
fn auto_network_dhcp_default_in_plan() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            auto = true
            disk-encryption = "none"
            disk = "largest-ssd"

            hostname = "test-host"
        "#,
        )
        .run()
        .read_plan();

    assert_eq!(
        plan["install_config"]["network"], "DHCP (all Ethernet interfaces)",
        "default network mode should be DHCP"
    );
}

// r[verify installer.finalise.network+4]
// r[verify installer.dryrun.schema+6]
#[test]
fn auto_network_static_in_plan() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            auto = true
            disk-encryption = "none"
            disk = "largest-ssd"

            hostname = "test-host"
            network-mode = "static"
            network-interface = "enp0s3"
            network-ip = "192.168.1.10/24"
            network-gateway = "192.168.1.1"
            network-dns = "8.8.8.8, 1.1.1.1"
        "#,
        )
        .run()
        .read_plan();

    let network = plan["install_config"]["network"].as_str().unwrap();
    assert!(
        network.starts_with("Static IP: 192.168.1.10/24 via 192.168.1.1 on enp0s3"),
        "expected static IP summary, got: {network}"
    );
    assert!(
        network.contains("8.8.8.8, 1.1.1.1"),
        "expected DNS in summary, got: {network}"
    );
}

// r[verify installer.finalise.network+4]
// r[verify installer.dryrun.schema+6]
#[test]
fn auto_network_offline_in_plan() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            auto = true
            disk-encryption = "none"
            disk = "largest-ssd"

            hostname = "test-host"
            network-mode = "offline"
        "#,
        )
        .run()
        .read_plan();

    assert_eq!(
        plan["install_config"]["network"],
        "Offline (no network configuration)",
    );
}

// r[verify installer.finalise.network+4]
// r[verify installer.dryrun.schema+6]
#[test]
fn auto_network_ipv6_slaac_in_plan() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            auto = true
            disk-encryption = "none"
            disk = "largest-ssd"

            hostname = "test-host"
            network-mode = "ipv6-slaac"
        "#,
        )
        .run()
        .read_plan();

    assert_eq!(plan["install_config"]["network"], "IPv6 SLAAC only",);
}
