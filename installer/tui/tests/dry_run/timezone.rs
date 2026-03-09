use super::common::{Fixture, SINGLE_SSD_DEVICE};

// r[verify installer.tui.timezone]
// r[verify installer.finalise.timezone]
// r[verify installer.dryrun.schema+6]
#[test]
fn auto_timezone_defaults_to_utc() {
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

    // When no install-time fields are set, install_config is null.
    // The effective timezone defaults to UTC, but there is no install_config
    // object to carry it.
    assert!(
        plan["install_config"].is_null(),
        "install_config should be null when no fields are configured"
    );
}

// r[verify installer.tui.timezone]
// r[verify installer.finalise.timezone]
// r[verify installer.config.timezone]
#[test]
fn auto_timezone_from_config() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            auto = true
            disk-encryption = "none"
            disk = "largest-ssd"

            timezone = "Pacific/Auckland"
        "#,
        )
        .run()
        .read_plan();

    assert_eq!(plan["install_config"]["timezone"], "Pacific/Auckland");
}

// r[verify installer.tui.timezone]
// r[verify installer.dryrun.schema+6]
#[test]
fn auto_encrypted_timezone_from_config() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .config(
            r#"
            auto = true
            disk-encryption = "tpm"
            disk = "largest-ssd"

            hostname = "tz-test"
            timezone = "America/New_York"
        "#,
        )
        .run()
        .read_plan();

    assert_eq!(plan["install_config"]["hostname"], "tz-test");
    assert_eq!(plan["install_config"]["timezone"], "America/New_York");
}

// r[verify installer.tui.timezone]
#[test]
fn scripted_timezone_search_and_select() {
    let f = Fixture::new();
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .timezones()
        .start_screen("timezone")
        .script(
            "\
# Timezone: search for 'auck', select first match
type:auck
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

    assert_eq!(plan["install_config"]["timezone"], "Pacific/Auckland");
}

// r[verify installer.tui.timezone]
#[test]
fn scripted_timezone_navigate_and_select() {
    let f = Fixture::new();
    // Timezones in sorted order: America/New_York(0), Europe/London(1),
    // Pacific/Auckland(2), UTC(3). Default cursor at UTC (index 3).
    let plan = f
        .scripted_run(SINGLE_SSD_DEVICE)
        .timezones()
        .start_screen("timezone")
        .script(
            "\
# Timezone: up twice from UTC(3) -> Europe/London(1), then select
up
up
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
