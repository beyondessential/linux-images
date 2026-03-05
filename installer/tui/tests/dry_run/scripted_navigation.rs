use super::common::{Fixture, SINGLE_SSD_DEVICE, TWO_DISK_DEVICES, installer};

// r[verify installer.tui.tpm-toggle]
// r[verify installer.tui.disk-encryption+2]
#[test]
fn scripted_tpm_toggle_twice_leaves_enabled() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    let script = f.write_script(
        "\
# Welcome
enter
# Disk
enter
# Variant
enter
# Toggle TPM disable on, then off again
space
space
enter
# Hostname selector: Static is default for metal, Enter -> HostnameInput
enter
# HostnameInput: type 'h' (required for metal)
type:h
enter
# Login: type password
type:pw
enter
type:pw
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
    assert!(!plan["tpm_present"].as_bool().unwrap());
}

// r[verify installer.tui.variant-selection]
#[test]
fn scripted_variant_toggle_back_to_metal() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    // Toggle variant twice (metal -> cloud -> metal), which means we get TpmToggle
    let script = f.write_script(
        "\
# Welcome
enter
# Disk
enter
# Toggle variant: metal->cloud
down
# Toggle variant: cloud->metal
down
enter
# Now we should be on TpmToggle (metal flow)
enter
# Hostname selector: Static is default for metal, Enter -> HostnameInput
enter
# HostnameInput: type 'h' (required for metal)
type:h
enter
# Login: type password
type:pw
enter
type:pw
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
    assert_eq!(plan["disk_encryption"], "tpm");
}

// r[verify installer.tui.disk-detection+3]
#[test]
fn scripted_disk_wrap_around() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    // Two devices: index 0 = nvme0n1, index 1 = sda.
    // Navigate down twice to wrap around back to index 0.
    let script = f.write_script(
        "\
# Welcome
enter
# Disk: down twice wraps back to index 0
down
down
enter
# Variant
enter
# TpmToggle
enter
# Hostname selector: Static is default for metal, Enter -> HostnameInput
enter
# HostnameInput: type 'h' (required for metal)
type:h
enter
# Login: type password
type:pw
enter
type:pw
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

// r[verify installer.tui.disk-detection+3]
#[test]
fn scripted_disk_up_wraps_to_last() {
    let f = Fixture::new();
    let devices = f.write_devices(TWO_DISK_DEVICES);
    // Starting at index 0, up wraps to index 1 (last device)
    let script = f.write_script(
        "\
# Welcome
enter
# Disk: up wraps to last
up
enter
# Variant
enter
# TpmToggle
enter
# Hostname selector: Static is default for metal, Enter -> HostnameInput
enter
# HostnameInput: type 'h' (required for metal)
type:h
enter
# Login: type password
type:pw
enter
type:pw
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

// r[verify installer.tui.hostname+5]
#[test]
fn scripted_hostname_with_backspace_correction() {
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
# Hostname selector: network-assigned is default for cloud, Up to select Static
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

// r[verify installer.tui.ssh-keys+5]
#[test]
fn scripted_multiline_ssh_keys() {
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
# Hostname selector: network-assigned is default for cloud, Enter -> Login
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

// r[verify installer.tui.confirmation+4]
#[test]
fn scripted_wrong_confirmation_does_not_advance() {
    let f = Fixture::new();
    let devices = f.write_devices(SINGLE_SSD_DEVICE);
    // Type "no" and press enter — should not advance past confirmation.
    // Then script ends — we should still be on confirmation screen, and
    // the plan should still be emitted from whatever state we have.
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
# Hostname selector: Static is default for metal, Enter -> HostnameInput
enter
# HostnameInput: type 'h' (required for metal)
type:h
enter
# Login: type password
type:pw
enter
type:pw
enter
# NetworkResults
enter
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
