use std::time::Duration;

// r[impl installer.tui.progress+3]
pub struct WriteProgress {
    pub bytes_written: u64,
    pub total_bytes: Option<u64>,
    pub elapsed: Duration,
}

impl WriteProgress {
    pub fn fraction(&self) -> Option<f64> {
        self.total_bytes
            .map(|total| self.bytes_written as f64 / total as f64)
    }

    pub fn eta(&self) -> Option<Duration> {
        let fraction = self.fraction()?;
        if fraction <= 0.0 {
            return None;
        }
        let total_estimated = self.elapsed.as_secs_f64() / fraction;
        let remaining = total_estimated - self.elapsed.as_secs_f64();
        if remaining < 0.0 {
            return Some(Duration::ZERO);
        }
        Some(Duration::from_secs_f64(remaining))
    }

    pub fn throughput_mbps(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64();
        if secs <= 0.0 {
            return 0.0;
        }
        (self.bytes_written as f64) / (1024.0 * 1024.0) / secs
    }
}

pub fn format_eta(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{:02}s", secs / 60, secs % 60)
    }
}

pub(crate) fn format_size(bytes: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify installer.tui.progress+3]
    #[test]
    fn progress_fraction_with_total() {
        let p = WriteProgress {
            bytes_written: 500,
            total_bytes: Some(1000),
            elapsed: Duration::from_secs(10),
        };
        assert!((p.fraction().unwrap() - 0.5).abs() < f64::EPSILON);
    }

    // r[verify installer.tui.progress+3]
    #[test]
    fn progress_fraction_without_total() {
        let p = WriteProgress {
            bytes_written: 500,
            total_bytes: None,
            elapsed: Duration::from_secs(10),
        };
        assert!(p.fraction().is_none());
    }

    // r[verify installer.tui.progress+3]
    #[test]
    fn progress_eta_calculation() {
        let p = WriteProgress {
            bytes_written: 500,
            total_bytes: Some(1000),
            elapsed: Duration::from_secs(10),
        };
        let eta = p.eta().unwrap();
        assert!((eta.as_secs_f64() - 10.0).abs() < 0.1);
    }

    // r[verify installer.tui.progress+3]
    #[test]
    fn progress_eta_at_zero() {
        let p = WriteProgress {
            bytes_written: 0,
            total_bytes: Some(1000),
            elapsed: Duration::from_secs(0),
        };
        assert!(p.eta().is_none());
    }

    // r[verify installer.tui.progress+3]
    #[test]
    fn progress_eta_complete() {
        let p = WriteProgress {
            bytes_written: 1000,
            total_bytes: Some(1000),
            elapsed: Duration::from_secs(10),
        };
        let eta = p.eta().unwrap();
        assert!(eta.as_secs_f64() < 0.1);
    }

    // r[verify installer.tui.progress+3]
    #[test]
    fn progress_throughput() {
        let p = WriteProgress {
            bytes_written: 10 * 1024 * 1024,
            total_bytes: None,
            elapsed: Duration::from_secs(1),
        };
        assert!((p.throughput_mbps() - 10.0).abs() < 0.1);
    }

    // r[verify installer.tui.progress+3]
    #[test]
    fn eta_formatting() {
        assert_eq!(format_eta(Duration::from_secs(45)), "45s");
        assert_eq!(format_eta(Duration::from_secs(90)), "1m30s");
        assert_eq!(format_eta(Duration::from_secs(3661)), "61m01s");
    }

    // r[verify installer.write.decompress-stream+2]
    #[test]
    fn size_formatting() {
        assert_eq!(format_size(0), "0.0 MiB");
        assert_eq!(format_size(1024 * 1024 * 512), "512.0 MiB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.00 GiB");
        assert_eq!(format_size(8 * 1024 * 1024 * 1024), "8.00 GiB");
    }
}
