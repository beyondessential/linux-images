use std::path::PathBuf;

use assert_cmd::cargo_bin_cmd;
use predicates::prelude::*;
use serde_json::Value;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let p = self.path(name);
        std::fs::write(&p, contents).unwrap();
        p
    }

    fn write_devices(&self, json: &str) -> PathBuf {
        self.write("devices.json", json)
    }

    fn write_config(&self, toml: &str) -> PathBuf {
        self.write("config.toml", toml)
    }

    fn write_script(&self, script: &str) -> PathBuf {
        self.write("script.txt", script)
    }

    fn plan_path(&self) -> PathBuf {
        self.path("plan.json")
    }

    fn log_path(&self) -> PathBuf {
        self.path("installer.log")
    }

    fn read_plan(&self) -> Value {
        let contents = std::fs::read_to_string(self.plan_path()).unwrap();
        serde_json::from_str(&contents).unwrap()
    }
}

const TWO_DISK_DEVICES: &str = r#"[
    {
        "path": "/dev/nvme0n1",
        "size_bytes": 1000204886016,
        "model": "Samsung 980 PRO",
        "transport": "Nvme"
    },
    {
        "path": "/dev/sda",
        "size_bytes": 500107862016,
        "model": "WD Blue",
        "transport": "Sata"
    }
]"#;

const SINGLE_SSD_DEVICE: &str = r#"[
    {
        "path": "/dev/nvme0n1",
        "size_bytes": 1000204886016,
        "model": "Samsung 980 PRO",
        "transport": "Nvme"
    }
]"#;

const THREE_MIXED_DEVICES: &str = r#"[
    {
        "path": "/dev/sda",
        "size_bytes": 2000000000000,
        "model": "Big HDD",
        "transport": "Sata"
    },
    {
        "path": "/dev/nvme0n1",
        "size_bytes": 500000000000,
        "model": "Small NVMe",
        "transport": "Nvme"
    },
    {
        "path": "/dev/nvme1n1",
        "size_bytes": 1000000000000,
        "model": "Big NVMe",
        "transport": "Nvme"
    }
]"#;

fn installer() -> assert_cmd::Command {
    cargo_bin_cmd!("bes-installer")
}

// ---------------------------------------------------------------------------
// Auto-mode tests
// ---------------------------------------------------------------------------

// r[verify installer.dryrun]
// r[verify installer.dryrun.output]
// r[verify installer.dryrun.schema]
// r[verify installer.mode.auto]
#[test]
fn auto_full_config_produces_correct_plan() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
        disk = "largest-ssd"

        [firstboot]
        hostname = "server-01"
        tailscale-authkey = "tskey-auth-xxxxx"
        ssh-authorized-keys = [
            "ssh-ed25519 AAAA admin@example.com",
            "ssh-rsa BBBB backup@example.com",
        ]
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
    assert_eq!(plan["mode"], "auto");
    assert_eq!(plan["variant"], "metal");
    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
    assert_eq!(plan["disk"]["model"], "Samsung 980 PRO");
    assert_eq!(plan["disk"]["size_bytes"], 1000204886016u64);
    assert_eq!(plan["disk"]["transport"], "NVMe");
    assert!(!plan["disable_tpm"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["hostname"], "server-01");
    assert!(plan["firstboot"]["tailscale_authkey"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["ssh_authorized_keys_count"], 2);
    assert!(plan["config_warnings"].as_array().unwrap().is_empty());
}

// r[verify installer.dryrun.schema]
// r[verify installer.config.schema]
#[test]
fn auto_disk_path_resolves_correctly() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    let config = f.write_config(
        r#"
        auto = true
        variant = "cloud"
        disk = "/dev/sda"
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
    assert_eq!(plan["mode"], "auto");
    assert_eq!(plan["variant"], "cloud");
    assert_eq!(plan["disk"]["path"], "/dev/sda");
    assert_eq!(plan["disk"]["model"], "WD Blue");
}

// r[verify installer.dryrun.schema]
// r[verify installer.config.schema]
#[test]
fn auto_disable_tpm_reflected_in_plan() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
        disk = "largest-ssd"
        disable-tpm = true
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
    assert!(plan["disable_tpm"].as_bool().unwrap());
}

// r[verify installer.dryrun.schema]
#[test]
fn auto_cloud_variant_no_firstboot() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "cloud"
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
    assert_eq!(plan["variant"], "cloud");
    assert!(plan["firstboot"].is_null());
    assert!(!plan["disable_tpm"].as_bool().unwrap());
}

// r[verify installer.config.schema]
#[test]
fn auto_disable_tpm_on_cloud_emits_warning() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "cloud"
        disk = "largest-ssd"
        disable-tpm = true
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
    let warnings = plan["config_warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap().contains("disable-tpm")),
        "expected a warning about disable-tpm, got: {warnings:?}"
    );
}

// r[verify installer.config.schema]
#[test]
fn auto_bad_hostname_emits_warning() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
        disk = "largest-ssd"

        [firstboot]
        hostname = "-invalid-"
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
    let warnings = plan["config_warnings"].as_array().unwrap();
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str().unwrap().contains("not a valid hostname")),
        "expected a hostname validation warning, got: {warnings:?}"
    );
}

// ---------------------------------------------------------------------------
// Auto-incomplete tests
// ---------------------------------------------------------------------------

// r[verify installer.mode.auto-incomplete]
#[test]
fn auto_incomplete_missing_variant_falls_back() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        disk = "largest-ssd"
    "#,
    );
    let script = f.write_script("enter\nenter\nenter\nenter\nenter\nenter\ntab\ntype:yes\nenter\n");

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
        .success()
        .stderr(predicates::str::contains("variant"));

    let plan = f.read_plan();
    assert_eq!(plan["mode"], "auto-incomplete");
}

// r[verify installer.mode.auto-incomplete]
#[test]
fn auto_incomplete_missing_disk_falls_back() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
    "#,
    );
    let script =
        f.write_script("enter\nenter\nenter\nenter\nenter\nenter\nenter\ntab\ntype:yes\nenter\n");

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
        .success()
        .stderr(predicates::str::contains("disk"));

    let plan = f.read_plan();
    assert_eq!(plan["mode"], "auto-incomplete");
}

// r[verify installer.mode.auto-incomplete]
#[test]
fn auto_incomplete_missing_both_falls_back() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config("auto = true\n");
    let script = f.write_script("enter\nenter\nenter\nenter\nenter\nenter\ntab\ntype:yes\nenter\n");

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
        .success()
        .stderr(predicates::str::contains("variant").and(predicates::str::contains("disk")));

    let plan = f.read_plan();
    assert_eq!(plan["mode"], "auto-incomplete");
}

// ---------------------------------------------------------------------------
// Prefilled mode tests
// ---------------------------------------------------------------------------

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
enter
enter
enter
enter
enter
enter
tab
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
    assert_eq!(plan["variant"], "metal");
    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
    assert_eq!(plan["firstboot"]["hostname"], "prefilled-host");
    assert!(plan["firstboot"]["tailscale_authkey"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["ssh_authorized_keys_count"], 1);
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
# SSH keys
tab
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

// ---------------------------------------------------------------------------
// Interactive mode tests (no config file)
// ---------------------------------------------------------------------------

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
# Tailscale
type:tskey-auth-test
enter
# SSH keys
type:ssh-ed25519 AAAA testkey
tab
# Confirm
type:yes
enter
",
    );

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
    assert!(plan["disable_tpm"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["hostname"], "my-server");
    assert!(plan["firstboot"]["tailscale_authkey"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["ssh_authorized_keys_count"], 1);
}

// r[verify installer.mode.interactive]
// r[verify installer.tui.variant-selection]
#[test]
fn interactive_cloud_skips_tpm_screen() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    // Cloud flow: welcome, disk, toggle variant to cloud, hostname, tailscale,
    // ssh, confirm. No TpmToggle screen.
    let script = f.write_script(
        "\
enter
enter
down
enter
enter
enter
tab
type:yes
enter
",
    );

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
    assert_eq!(plan["variant"], "cloud");
    assert!(!plan["disable_tpm"].as_bool().unwrap());
}

// r[verify installer.tui.hostname]
// r[verify installer.tui.tailscale]
// r[verify installer.tui.ssh-keys]
#[test]
fn interactive_firstboot_fields_captured() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let script = f.write_script(
        "\
enter
enter
down
enter
type:my-host
enter
type:tskey-auth-mykey
enter
type:ssh-ed25519 AAAA key1
enter
type:ssh-rsa BBBB key2
tab
type:yes
enter
",
    );

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
    assert_eq!(plan["firstboot"]["hostname"], "my-host");
    assert!(plan["firstboot"]["tailscale_authkey"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["ssh_authorized_keys_count"], 2);
}

// r[verify installer.tui.hostname]
// r[verify installer.tui.tailscale]
// r[verify installer.tui.ssh-keys]
#[test]
fn interactive_empty_firstboot_is_null() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    // Skip all optional firstboot fields
    let script = f.write_script(
        "\
enter
enter
enter
enter
enter
enter
tab
type:yes
enter
",
    );

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
    assert!(plan["firstboot"].is_null());
}

// r[verify installer.tui.disk-detection]
#[test]
fn interactive_selects_second_disk() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    let script = f.write_script(
        "\
enter
# Navigate to second disk
down
enter
enter
enter
enter
enter
tab
type:yes
enter
",
    );

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

// r[verify installer.tui.confirmation]
#[test]
fn interactive_go_back_from_confirmation_and_change() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    // Walk to confirmation, go back to ssh keys, go back to tailscale,
    // type a key, then walk forward again and confirm.
    let script = f.write_script(
        "\
enter
enter
down
enter
enter
enter
tab
# Now on Confirmation, go back
esc
# Back on SshKeys, go back again
esc
# Back on Tailscale, enter an authkey
type:tskey-auth-late
enter
# SshKeys, skip
tab
# Confirmation
type:yes
enter
",
    );

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
    assert!(plan["firstboot"]["tailscale_authkey"].as_bool().unwrap());
}

// ---------------------------------------------------------------------------
// Disk selection strategy tests (auto mode)
// ---------------------------------------------------------------------------

// r[verify installer.config.schema]
#[test]
fn strategy_largest_ssd_picks_biggest_nvme() {
    let f = Fixture::new();
    let devices = f.write_devices(THREE_MIXED_DEVICES);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
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
    assert_eq!(plan["disk"]["path"], "/dev/nvme1n1");
    assert_eq!(plan["disk"]["model"], "Big NVMe");
}

// r[verify installer.config.schema]
#[test]
fn strategy_largest_picks_biggest_overall() {
    let f = Fixture::new();
    let devices = f.write_devices(THREE_MIXED_DEVICES);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
        disk = "largest"
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
    assert_eq!(plan["disk"]["path"], "/dev/sda");
    assert_eq!(plan["disk"]["size_bytes"], 2000000000000u64);
}

// r[verify installer.config.schema]
#[test]
fn strategy_smallest_picks_smallest_overall() {
    let f = Fixture::new();
    let devices = f.write_devices(THREE_MIXED_DEVICES);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
        disk = "smallest"
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
    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
    assert_eq!(plan["disk"]["size_bytes"], 500000000000u64);
}

// r[verify installer.config.schema]
#[test]
fn strategy_disk_path_selects_exact_device() {
    let f = Fixture::new();
    let devices = f.write_devices(THREE_MIXED_DEVICES);
    let config = f.write_config(
        r#"
        auto = true
        variant = "cloud"
        disk = "/dev/nvme1n1"
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
    assert_eq!(plan["disk"]["path"], "/dev/nvme1n1");
    assert_eq!(plan["disk"]["model"], "Big NVMe");
}

// ---------------------------------------------------------------------------
// Error handling tests
// ---------------------------------------------------------------------------

// r[verify installer.dryrun.devices]
#[test]
fn error_no_devices_file() {
    let f = Fixture::new();
    let bad_path = f.path("nonexistent.json");

    installer()
        .args([
            "--fake-devices",
            bad_path.to_str().unwrap(),
            "--dry-run",
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("fake devices"));
}

#[test]
fn error_empty_devices_list() {
    let f = Fixture::new();
    let devices = f.write_devices("[]");

    installer()
        .args([
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("no block devices"));
}

// r[verify installer.dryrun.devices]
#[test]
fn error_invalid_devices_json() {
    let f = Fixture::new();
    let devices = f.write("devices.json", "this is not json");

    installer()
        .args([
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("fake devices"));
}

// r[verify installer.config.schema]
#[test]
fn error_invalid_config_toml() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config("this is not valid toml {{{{");

    installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("parsing config"));
}

// r[verify installer.config.schema]
#[test]
fn error_unknown_config_field() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config("bogus = true\n");

    installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("parsing config"));
}

// r[verify installer.config.schema]
#[test]
fn error_invalid_variant_in_config() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(r#"variant = "turbo""#);

    installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure();
}

// r[verify installer.dryrun.script]
#[test]
fn error_bad_script_token() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let script = f.write_script("enter\nfoobar\n");

    installer()
        .args([
            "--fake-devices",
            devices.to_str().unwrap(),
            "--input-script",
            script.to_str().unwrap(),
            "--dry-run",
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("foobar"));
}

// r[verify installer.dryrun.script]
#[test]
fn error_nonexistent_script_file() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let bad_path = f.path("nonexistent.script");

    installer()
        .args([
            "--fake-devices",
            devices.to_str().unwrap(),
            "--input-script",
            bad_path.to_str().unwrap(),
            "--dry-run",
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("script"));
}

#[test]
fn error_disk_path_not_found() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
        disk = "/dev/nonexistent"
    "#,
    );

    installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("not found"));
}

#[test]
fn error_no_ssds_for_largest_ssd_strategy() {
    let f = Fixture::new();
    // Only SATA disks, no SSDs
    let devices = f.write_devices(
        r#"[{"path": "/dev/sda", "size_bytes": 1000000000000, "model": "HDD", "transport": "Sata"}]"#,
    );
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
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
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicates::str::contains("SSD"));
}

// ---------------------------------------------------------------------------
// Output destination tests
// ---------------------------------------------------------------------------

// r[verify installer.dryrun.output]
#[test]
fn dry_run_output_to_file() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
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
        .success()
        .stdout(predicates::str::is_empty());

    let plan = f.read_plan();
    assert_eq!(plan["mode"], "auto");
}

// r[verify installer.dryrun.output]
#[test]
fn dry_run_output_to_stdout() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
        disk = "largest-ssd"
    "#,
    );

    let output = installer()
        .args([
            "--config",
            config.to_str().unwrap(),
            "--fake-devices",
            devices.to_str().unwrap(),
            "--dry-run",
            "--log",
            f.log_path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let plan: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(plan["mode"], "auto");
    assert_eq!(plan["variant"], "metal");
}

// ---------------------------------------------------------------------------
// Schema completeness tests
// ---------------------------------------------------------------------------

// r[verify installer.dryrun.schema]
#[test]
fn plan_contains_all_required_fields() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
        disk = "largest-ssd"
        disable-tpm = true

        [firstboot]
        hostname = "test-box"
        tailscale-authkey = "tskey-auth-xxx"
        ssh-authorized-keys = ["ssh-ed25519 AAAA k1"]
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
    let obj = plan.as_object().unwrap();

    let required_top = [
        "mode",
        "variant",
        "disk",
        "disable_tpm",
        "firstboot",
        "image_path",
        "config_warnings",
    ];
    for key in &required_top {
        assert!(obj.contains_key(*key), "missing top-level key: {key}");
    }

    let disk = plan["disk"].as_object().unwrap();
    for key in &["path", "model", "size_bytes", "transport"] {
        assert!(disk.contains_key(*key), "missing disk key: {key}");
    }

    let fb = plan["firstboot"].as_object().unwrap();
    for key in &["hostname", "tailscale_authkey", "ssh_authorized_keys_count"] {
        assert!(fb.contains_key(*key), "missing firstboot key: {key}");
    }
}

// r[verify installer.dryrun.schema]
#[test]
fn plan_tailscale_authkey_is_bool_not_string() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
        disk = "largest-ssd"

        [firstboot]
        tailscale-authkey = "tskey-auth-secret-should-not-appear"
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
    assert!(plan["firstboot"]["tailscale_authkey"].is_boolean());
    assert!(plan["firstboot"]["tailscale_authkey"].as_bool().unwrap());

    let raw = std::fs::read_to_string(f.plan_path()).unwrap();
    assert!(
        !raw.contains("tskey-auth-secret-should-not-appear"),
        "authkey secret leaked into plan output"
    );
}

// ---------------------------------------------------------------------------
// Dry-run without input-script (prefilled mode, initial state)
// ---------------------------------------------------------------------------

// r[verify installer.dryrun]
#[test]
fn dry_run_without_script_emits_initial_state() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    let config = f.write_config(
        r#"
        variant = "cloud"
        disk = "/dev/sda"

        [firstboot]
        hostname = "pre-host"
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
    assert_eq!(plan["mode"], "prefilled");
    assert_eq!(plan["variant"], "cloud");
    assert_eq!(plan["disk"]["path"], "/dev/sda");
    assert_eq!(plan["firstboot"]["hostname"], "pre-host");
}

// r[verify installer.mode.interactive]
#[test]
fn dry_run_no_config_is_interactive() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);

    installer()
        .args([
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
    assert_eq!(plan["mode"], "interactive");
    assert_eq!(plan["variant"], "metal");
    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
}

// ---------------------------------------------------------------------------
// Fake devices format tests
// ---------------------------------------------------------------------------

// r[verify installer.dryrun.devices]
#[test]
fn fake_devices_removable_field_optional() {
    let f = Fixture::new();
    let devices = f.write_devices(
        r#"[{"path": "/dev/sda", "size_bytes": 100000000000, "model": "Test", "transport": "Nvme"}]"#,
    );
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
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
    assert_eq!(plan["disk"]["path"], "/dev/sda");
}

// r[verify installer.dryrun.devices]
#[test]
fn fake_devices_transport_aliases_accepted() {
    let f = Fixture::new();
    // Use display-form aliases: "NVMe", "SATA", etc.
    let devices = f.write_devices(
        r#"[
            {"path": "/dev/nvme0n1", "size_bytes": 100000000000, "model": "A", "transport": "NVMe"},
            {"path": "/dev/sda", "size_bytes": 200000000000, "model": "B", "transport": "SATA"}
        ]"#,
    );
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
        disk = "largest"
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
    assert_eq!(plan["disk"]["path"], "/dev/sda");
}

// ---------------------------------------------------------------------------
// Config edge cases
// ---------------------------------------------------------------------------

// r[verify installer.config.schema]
#[test]
fn empty_config_file_treated_as_prefilled() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config("");

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
    // Empty config has auto=false by default -> prefilled mode
    assert_eq!(plan["mode"], "prefilled");
}

// r[verify installer.config.schema]
#[test]
fn auto_with_minimal_firstboot() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "cloud"
        disk = "largest-ssd"

        [firstboot]
        hostname = "just-a-hostname"
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
    assert_eq!(plan["firstboot"]["hostname"], "just-a-hostname");
    assert!(!plan["firstboot"]["tailscale_authkey"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["ssh_authorized_keys_count"], 0);
}

// r[verify installer.config.schema]
#[test]
fn auto_with_only_ssh_keys() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
        disk = "largest-ssd"

        [firstboot]
        ssh-authorized-keys = [
            "ssh-ed25519 AAAA k1",
            "ssh-ed25519 BBBB k2",
            "ssh-rsa CCCC k3",
        ]
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
    assert!(plan["firstboot"]["hostname"].is_null());
    assert!(!plan["firstboot"]["tailscale_authkey"].as_bool().unwrap());
    assert_eq!(plan["firstboot"]["ssh_authorized_keys_count"], 3);
}

// ---------------------------------------------------------------------------
// Scripted TUI: navigation edge cases
// ---------------------------------------------------------------------------

// r[verify installer.tui.tpm-toggle]
#[test]
fn scripted_tpm_toggle_twice_leaves_enabled() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let script = f.write_script(
        "\
enter
enter
enter
# Toggle TPM disable on, then off again
space
space
enter
enter
enter
tab
type:yes
enter
",
    );

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
    assert!(!plan["disable_tpm"].as_bool().unwrap());
}

// r[verify installer.tui.variant-selection]
#[test]
fn scripted_variant_toggle_back_to_metal() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    // Toggle variant twice (metal -> cloud -> metal), which means we get TpmToggle
    let script = f.write_script(
        "\
enter
enter
# Toggle variant: metal->cloud
down
# Toggle variant: cloud->metal
down
enter
# Now we should be on TpmToggle (metal flow)
enter
enter
enter
tab
type:yes
enter
",
    );

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
    assert_eq!(plan["variant"], "metal");
}

// r[verify installer.tui.disk-detection]
#[test]
fn scripted_disk_wrap_around() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    // Two devices: index 0 = nvme0n1, index 1 = sda.
    // Navigate down twice to wrap around back to index 0.
    let script = f.write_script(
        "\
enter
down
down
enter
enter
enter
enter
enter
tab
type:yes
enter
",
    );

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
    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
}

// r[verify installer.tui.disk-detection]
#[test]
fn scripted_disk_up_wraps_to_last() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    // Starting at index 0, up wraps to index 1 (last device)
    let script = f.write_script(
        "\
enter
up
enter
enter
enter
enter
enter
tab
type:yes
enter
",
    );

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
    assert_eq!(plan["disk"]["path"], "/dev/sda");
}

// r[verify installer.tui.hostname]
#[test]
fn scripted_hostname_with_backspace_correction() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let script = f.write_script(
        "\
enter
enter
down
enter
# Hostname: type 'baaad', backspace 3 times, type 'd'
type:baaad
backspace
backspace
backspace
type:d
enter
enter
tab
type:yes
enter
",
    );

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
    assert_eq!(plan["firstboot"]["hostname"], "bad");
}

// r[verify installer.tui.ssh-keys]
#[test]
fn scripted_multiline_ssh_keys() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let script = f.write_script(
        "\
enter
enter
down
enter
# Hostname: skip
enter
# Tailscale: skip
enter
# SSH keys: two keys separated by Enter (newline in SSH keys screen)
type:ssh-ed25519 AAAA key1
enter
type:ssh-rsa BBBB key2
tab
type:yes
enter
",
    );

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
    assert_eq!(plan["firstboot"]["ssh_authorized_keys_count"], 2);
}

// r[verify installer.tui.confirmation]
#[test]
fn scripted_wrong_confirmation_does_not_advance() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    // Type "no" and press enter — should not advance past confirmation.
    // Then script ends — we should still be on confirmation screen, and
    // the plan should still be emitted from whatever state we have.
    let script = f.write_script(
        "\
enter
enter
enter
enter
enter
enter
tab
type:no
enter
",
    );

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

    // Plan is still produced (from current state at script end)
    let plan = f.read_plan();
    assert_eq!(plan["mode"], "interactive");
}

// ---------------------------------------------------------------------------
// Multiple config warnings
// ---------------------------------------------------------------------------

// r[verify installer.config.schema]
#[test]
fn multiple_validation_warnings_collected() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "cloud"
        disk = "largest-ssd"
        disable-tpm = true

        [firstboot]
        hostname = "-bad-"
        ssh-authorized-keys = [""]
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
    let warnings = plan["config_warnings"].as_array().unwrap();
    assert!(
        warnings.len() >= 3,
        "expected at least 3 warnings (disable-tpm, bad hostname, empty ssh key), got: {warnings:?}"
    );
}

// ---------------------------------------------------------------------------
// image_path is null in dry-run (no images on disk)
// ---------------------------------------------------------------------------

// r[verify installer.dryrun]
#[test]
fn dry_run_image_path_is_null_when_no_images() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let config = f.write_config(
        r#"
        auto = true
        variant = "metal"
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
    assert!(
        plan["image_path"].is_null(),
        "image_path should be null in dry-run without actual images"
    );
}
