use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const ZONE1970_TAB: &str = "/usr/share/zoneinfo/zone1970.tab";

/// r[impl installer.tui.timezone]
/// Load IANA timezone names from the system's zone1970.tab file.
/// Falls back to a minimal list containing just "UTC" if the file
/// cannot be read.
pub fn load_system_timezones() -> Vec<String> {
    match parse_zone1970_tab(&PathBuf::from(ZONE1970_TAB)) {
        Ok(zones) if !zones.is_empty() => zones,
        Ok(_) => {
            tracing::warn!("zone1970.tab was empty, using fallback");
            fallback_timezones()
        }
        Err(e) => {
            tracing::warn!("failed to read zone1970.tab: {e:#}, using fallback");
            fallback_timezones()
        }
    }
}

/// r[impl installer.tui.timezone]
/// Load timezone names from a text file (one per line) for testing.
pub fn load_from_file(path: &Path) -> Result<Vec<String>> {
    let contents =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let zones: Vec<String> = contents
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect();
    if zones.is_empty() {
        anyhow::bail!("no timezones found in {}", path.display());
    }
    Ok(zones)
}

/// Parse zone1970.tab, extracting the timezone name (column 3) from each
/// non-comment line.
fn parse_zone1970_tab(path: &Path) -> Result<Vec<String>> {
    let contents =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut zones: Vec<String> = contents
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .filter_map(|l| {
            let mut cols = l.split('\t');
            let _countries = cols.next()?;
            let _coords = cols.next()?;
            let tz = cols.next()?;
            Some(tz.trim().to_string())
        })
        .collect();
    zones.sort();
    zones.dedup();
    if !zones.iter().any(|z| z == "UTC") {
        zones.push("UTC".to_string());
        zones.sort();
    }
    Ok(zones)
}

fn fallback_timezones() -> Vec<String> {
    vec!["UTC".to_string()]
}

/// Validate that a timezone name exists in the available list.
#[cfg(test)]
pub fn is_valid(tz: &str, available: &[String]) -> bool {
    available.iter().any(|z| z == tz)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_zone1970_tab_from_system() {
        let path = PathBuf::from(ZONE1970_TAB);
        if !path.exists() {
            return;
        }
        let zones = parse_zone1970_tab(&path).unwrap();
        assert!(!zones.is_empty());
        assert!(zones.iter().any(|z| z == "UTC" || z == "Etc/UTC"));
        assert!(zones.iter().any(|z| z == "America/New_York"));
        assert!(zones.iter().any(|z| z == "Europe/London"));
    }

    #[test]
    fn load_system_always_returns_something() {
        let zones = load_system_timezones();
        assert!(!zones.is_empty());
    }

    #[test]
    fn load_from_file_works() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zones.txt");
        std::fs::write(
            &path,
            "UTC\nAmerica/New_York\nEurope/London\nPacific/Auckland\n",
        )
        .unwrap();
        let zones = load_from_file(&path).unwrap();
        assert_eq!(zones.len(), 4);
        assert_eq!(zones[0], "UTC");
        assert_eq!(zones[1], "America/New_York");
    }

    #[test]
    fn load_from_file_skips_comments_and_blanks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zones.txt");
        std::fs::write(&path, "# comment\n\nUTC\n  \nAsia/Tokyo\n").unwrap();
        let zones = load_from_file(&path).unwrap();
        assert_eq!(zones, vec!["UTC", "Asia/Tokyo"]);
    }

    #[test]
    fn load_from_file_empty_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zones.txt");
        std::fs::write(&path, "# only comments\n").unwrap();
        assert!(load_from_file(&path).is_err());
    }

    #[test]
    fn is_valid_checks_list() {
        let available = vec![
            "UTC".to_string(),
            "America/New_York".to_string(),
            "Europe/London".to_string(),
        ];
        assert!(is_valid("UTC", &available));
        assert!(is_valid("America/New_York", &available));
        assert!(!is_valid("Mars/Olympus_Mons", &available));
    }

    #[test]
    fn parse_zone1970_tab_synthetic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zone1970.tab");
        std::fs::write(
            &path,
            "\
# tzdb timezone descriptions
US\t+404251-0740023\tAmerica/New_York\tEastern
GB\t+513030-0000731\tEurope/London
NZ\t-3652+17446\tPacific/Auckland\tMost locations
",
        )
        .unwrap();
        let zones = parse_zone1970_tab(&path).unwrap();
        assert!(zones.contains(&"America/New_York".to_string()));
        assert!(zones.contains(&"Europe/London".to_string()));
        assert!(zones.contains(&"Pacific/Auckland".to_string()));
        assert!(zones.contains(&"UTC".to_string()));
    }

    #[test]
    fn zones_are_sorted_and_deduped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zone1970.tab");
        std::fs::write(
            &path,
            "\
US\t+1\tAmerica/New_York
GB\t+2\tEurope/London
US\t+3\tAmerica/New_York
",
        )
        .unwrap();
        let zones = parse_zone1970_tab(&path).unwrap();
        let ny_count = zones
            .iter()
            .filter(|z| z.as_str() == "America/New_York")
            .count();
        assert_eq!(ny_count, 1);
        let ny_pos = zones.iter().position(|z| z == "America/New_York").unwrap();
        let london_pos = zones.iter().position(|z| z == "Europe/London").unwrap();
        assert!(ny_pos < london_pos);
    }
}
