use super::common::{Fixture, THREE_MIXED_DEVICES};

// r[verify installer.config.disk]
#[test]
fn strategy_largest_ssd_picks_biggest_nvme() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(THREE_MIXED_DEVICES)
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

    assert_eq!(plan["disk"]["path"], "/dev/nvme1n1");
    assert_eq!(plan["disk"]["model"], "Big NVMe");
}

// r[verify installer.config.disk]
#[test]
fn strategy_largest_picks_biggest_overall() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(THREE_MIXED_DEVICES)
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
    assert_eq!(plan["disk"]["size_bytes"], 2000000000000u64);
}

// r[verify installer.config.disk]
#[test]
fn strategy_smallest_picks_smallest_overall() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(THREE_MIXED_DEVICES)
        .config(
            r#"
            auto = true
            disk-encryption = "tpm"
            disk = "smallest"

            hostname = "test-host"
        "#,
        )
        .run()
        .read_plan();

    assert_eq!(plan["disk"]["path"], "/dev/nvme0n1");
    assert_eq!(plan["disk"]["size_bytes"], 500000000000u64);
}

// r[verify installer.config.disk]
#[test]
fn strategy_disk_path_selects_exact_device() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(THREE_MIXED_DEVICES)
        .config(
            r#"
            auto = true
            disk-encryption = "none"
            disk = "/dev/nvme1n1"
        "#,
        )
        .run()
        .read_plan();

    assert_eq!(plan["disk"]["path"], "/dev/nvme1n1");
    assert_eq!(plan["disk"]["model"], "Big NVMe");
}
