// r[impl installer.write.decompress-stream]
// r[impl installer.tui.progress]

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};

pub struct WriteProgress {
    pub bytes_written: u64,
    pub total_bytes: Option<u64>,
    pub elapsed: std::time::Duration,
}

impl WriteProgress {
    pub fn fraction(&self) -> Option<f64> {
        self.total_bytes
            .map(|total| self.bytes_written as f64 / total as f64)
    }

    pub fn eta(&self) -> Option<std::time::Duration> {
        let fraction = self.fraction()?;
        if fraction <= 0.0 {
            return None;
        }
        let total_estimated = self.elapsed.as_secs_f64() / fraction;
        let remaining = total_estimated - self.elapsed.as_secs_f64();
        if remaining < 0.0 {
            return Some(std::time::Duration::ZERO);
        }
        Some(std::time::Duration::from_secs_f64(remaining))
    }

    pub fn throughput_mbps(&self) -> f64 {
        let secs = self.elapsed.as_secs_f64();
        if secs <= 0.0 {
            return 0.0;
        }
        (self.bytes_written as f64) / (1024.0 * 1024.0) / secs
    }
}

pub fn format_eta(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m{:02}s", secs / 60, secs % 60)
    }
}

// r[impl installer.write.source]
pub fn find_image_path(variant: &str, arch: &str) -> Result<std::path::PathBuf> {
    let search_dirs = [
        "/run/live/medium/images",
        "/run/live/medium",
        "/cdrom/images",
        "/cdrom",
    ];

    let pattern = format!("{variant}-{arch}");

    for dir in &search_dirs {
        let dir_path = Path::new(dir);
        if !dir_path.is_dir() {
            continue;
        }
        let entries =
            std::fs::read_dir(dir_path).with_context(|| format!("reading directory {dir}"))?;
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.contains(&pattern) && name.ends_with(".raw.zst") {
                return Ok(entry.path());
            }
        }
    }

    bail!("no .raw.zst image found for variant={variant} arch={arch}");
}

/// Stream-decompress a `.raw.zst` file directly to a block device, calling `on_progress`
/// periodically with current write progress.
pub fn write_image(
    source: &Path,
    target: &Path,
    on_progress: &mut dyn FnMut(&WriteProgress),
) -> Result<()> {
    let compressed_size = std::fs::metadata(source)
        .with_context(|| format!("stat {}", source.display()))?
        .len();

    // Estimate uncompressed size from the compressed size. zstd typically achieves
    // 3-5x compression on disk images; we use 4x as a rough estimate. The progress
    // bar will adjust if we overshoot.
    let estimated_total = compressed_size * 4;

    let input =
        File::open(source).with_context(|| format!("opening source image {}", source.display()))?;

    let mut decoder = zstd::Decoder::new(input).context("initializing zstd decoder")?;

    let mut output = OpenOptions::new()
        .write(true)
        .open(target)
        .with_context(|| format!("opening target device {}", target.display()))?;

    let mut buf = vec![0u8; 4 * 1024 * 1024]; // 4 MiB buffer
    let mut bytes_written: u64 = 0;
    let start = Instant::now();

    loop {
        let n = decoder.read(&mut buf).context("reading from zstd stream")?;
        if n == 0 {
            break;
        }
        output
            .write_all(&buf[..n])
            .context("writing to target device")?;
        bytes_written += n as u64;

        on_progress(&WriteProgress {
            bytes_written,
            total_bytes: Some(estimated_total),
            elapsed: start.elapsed(),
        });
    }

    output.flush().context("flushing target device")?;

    // Sync to ensure all data is physically written
    sync_device(&output)?;

    // Final progress callback with actual total
    on_progress(&WriteProgress {
        bytes_written,
        total_bytes: Some(bytes_written),
        elapsed: start.elapsed(),
    });

    log::info!(
        "wrote {} to {} in {:.1}s ({:.1} MiB/s)",
        format_size(bytes_written),
        target.display(),
        start.elapsed().as_secs_f64(),
        (bytes_written as f64) / (1024.0 * 1024.0) / start.elapsed().as_secs_f64(),
    );

    Ok(())
}

fn sync_device(file: &File) -> Result<()> {
    file.sync_all().context("syncing target device")?;
    Ok(())
}

fn format_size(bytes: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    if bytes >= GIB {
        format!("{:.2} GiB", bytes as f64 / GIB as f64)
    } else {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    }
}

// r[impl installer.write.verify]
pub fn verify_partition_table(target: &Path) -> Result<()> {
    let output = std::process::Command::new("lsblk")
        .args([
            "--json",
            "--output",
            "NAME,PARTLABEL,PARTTYPENAME",
            target.to_str().unwrap_or_default(),
        ])
        .output()
        .context("running lsblk to verify partitions")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("lsblk verification failed: {stderr}");
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    let expected_labels = ["efi", "xboot", "root"];
    for label in &expected_labels {
        if !stdout.contains(label) {
            bail!(
                "partition verification failed: expected partition with label '{label}' not found"
            );
        }
    }

    log::info!("partition table verified on {}", target.display());
    Ok(())
}

/// Re-read the partition table on the target device so the kernel picks up changes.
pub fn reread_partition_table(target: &Path) -> Result<()> {
    let status = std::process::Command::new("partprobe")
        .arg(target)
        .status()
        .context("running partprobe")?;

    if !status.success() {
        bail!("partprobe failed on {}", target.display());
    }

    let _ = std::process::Command::new("udevadm")
        .args(["settle", "--timeout=5"])
        .status();

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // r[verify installer.tui.progress]
    #[test]
    fn progress_fraction_with_total() {
        let p = WriteProgress {
            bytes_written: 500,
            total_bytes: Some(1000),
            elapsed: std::time::Duration::from_secs(10),
        };
        assert!((p.fraction().unwrap() - 0.5).abs() < f64::EPSILON);
    }

    // r[verify installer.tui.progress]
    #[test]
    fn progress_fraction_without_total() {
        let p = WriteProgress {
            bytes_written: 500,
            total_bytes: None,
            elapsed: std::time::Duration::from_secs(10),
        };
        assert!(p.fraction().is_none());
    }

    // r[verify installer.tui.progress]
    #[test]
    fn progress_eta_calculation() {
        let p = WriteProgress {
            bytes_written: 500,
            total_bytes: Some(1000),
            elapsed: std::time::Duration::from_secs(10),
        };
        let eta = p.eta().unwrap();
        assert!((eta.as_secs_f64() - 10.0).abs() < 0.1);
    }

    // r[verify installer.tui.progress]
    #[test]
    fn progress_eta_at_zero() {
        let p = WriteProgress {
            bytes_written: 0,
            total_bytes: Some(1000),
            elapsed: std::time::Duration::from_secs(0),
        };
        assert!(p.eta().is_none());
    }

    // r[verify installer.tui.progress]
    #[test]
    fn progress_eta_complete() {
        let p = WriteProgress {
            bytes_written: 1000,
            total_bytes: Some(1000),
            elapsed: std::time::Duration::from_secs(10),
        };
        let eta = p.eta().unwrap();
        assert!(eta.as_secs_f64() < 0.1);
    }

    // r[verify installer.tui.progress]
    #[test]
    fn progress_throughput() {
        let p = WriteProgress {
            bytes_written: 10 * 1024 * 1024,
            total_bytes: None,
            elapsed: std::time::Duration::from_secs(1),
        };
        assert!((p.throughput_mbps() - 10.0).abs() < 0.1);
    }

    // r[verify installer.tui.progress]
    #[test]
    fn eta_formatting() {
        assert_eq!(format_eta(std::time::Duration::from_secs(45)), "45s");
        assert_eq!(format_eta(std::time::Duration::from_secs(90)), "1m30s");
        assert_eq!(format_eta(std::time::Duration::from_secs(3661)), "61m01s");
    }

    // r[verify installer.write.decompress-stream]
    #[test]
    fn size_formatting() {
        assert_eq!(format_size(0), "0.0 MiB");
        assert_eq!(format_size(1024 * 1024 * 512), "512.0 MiB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.00 GiB");
        assert_eq!(format_size(8 * 1024 * 1024 * 1024), "8.00 GiB");
    }
}
