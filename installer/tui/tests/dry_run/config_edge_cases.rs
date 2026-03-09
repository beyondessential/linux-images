use super::common::{Fixture, SINGLE_SSD_DEVICE};

// r[verify installer.dryrun.devices]
#[test]
fn fake_devices_removable_field_optional() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(
            r#"[{"path": "/dev/sda", "size_bytes": 100000000000, "model": "Test", "transport": "Nvme"}]"#,
        )
        .config(
            r#"
            auto = true
            disk-encryption = "tpm"
            disk = "largest-ssd"

            hostname = "test-host"
        "#,
        )
        .run()
        .read_plan();

    assert_eq!(plan["disk"]["path"], "/dev/sda");
}

// r[verify installer.dryrun.devices]
#[test]
fn fake_devices_transport_aliases_accepted() {
    let f = Fixture::new();
    // Use display-form aliases: "NVMe", "SATA", etc.
    let plan = f
        .scripted_run(
            r#"[
                {"path": "/dev/nvme0n1", "size_bytes": 100000000000, "model": "A", "transport": "NVMe"},
                {"path": "/dev/sda", "size_bytes": 200000000000, "model": "B", "transport": "SATA"}
            ]"#,
        )
        .config(
            r#"
            auto = true
            disk-encryption = "tpm"
            disk = "largest"

            hostname = "test-host"
        "#,
        )
        .run()
        .read_plan();

    assert_eq!(plan["disk"]["path"], "/dev/sda");
}

// r[verify installer.config.format]
#[test]
fn empty_config_file_treated_as_prefilled() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .config("")
        .run()
        .read_plan();

    // Empty config has auto=false by default -> prefilled mode
    assert_eq!(plan["mode"], "prefilled");
}
