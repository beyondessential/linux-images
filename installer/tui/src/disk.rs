use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::config::{DiskSelector, DiskStrategy};

// r[impl installer.dryrun.devices]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockDevice {
    pub path: PathBuf,
    pub size_bytes: u64,
    pub model: String,
    pub transport: TransportType,
    #[serde(default)]
    pub removable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportType {
    #[serde(alias = "NVMe", alias = "nvme")]
    Nvme,
    #[serde(alias = "SATA", alias = "sata")]
    Sata,
    #[serde(alias = "USB", alias = "usb")]
    Usb,
    #[serde(alias = "virtio")]
    Virtio,
    #[serde(alias = "SCSI", alias = "scsi")]
    Scsi,
    #[serde(alias = "unknown")]
    Unknown,
}

impl std::fmt::Display for TransportType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportType::Nvme => write!(f, "NVMe"),
            TransportType::Sata => write!(f, "SATA"),
            TransportType::Usb => write!(f, "USB"),
            TransportType::Virtio => write!(f, "virtio"),
            TransportType::Scsi => write!(f, "SCSI"),
            TransportType::Unknown => write!(f, "unknown"),
        }
    }
}

impl TransportType {
    fn from_tran(s: &str) -> Self {
        match s {
            "nvme" => TransportType::Nvme,
            "sata" | "ata" => TransportType::Sata,
            "usb" => TransportType::Usb,
            "virtio" => TransportType::Virtio,
            "scsi" | "sas" | "fc" | "iscsi" => TransportType::Scsi,
            _ => TransportType::Unknown,
        }
    }

    pub fn is_ssd(&self) -> bool {
        matches!(self, TransportType::Nvme | TransportType::Virtio)
    }
}

impl BlockDevice {
    pub fn is_ssd(&self) -> bool {
        self.transport.is_ssd() || self.rota_from_sysfs() == Some(false)
    }

    fn rota_from_sysfs(&self) -> Option<bool> {
        let name = self.path.file_name()?.to_str()?;
        let rota_path = format!("/sys/block/{name}/queue/rotational");
        let contents = std::fs::read_to_string(rota_path).ok()?;
        match contents.trim() {
            "0" => Some(false),
            "1" => Some(true),
            _ => None,
        }
    }

    pub fn size_display(&self) -> String {
        format_bytes(self.size_bytes)
    }
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;
    const TIB: u64 = GIB * 1024;

    if bytes >= TIB {
        format!("{:.1} TiB", bytes as f64 / TIB as f64)
    } else if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[derive(Deserialize)]
struct LsblkOutput {
    blockdevices: Vec<LsblkDevice>,
}

#[derive(Deserialize)]
struct LsblkDevice {
    name: String,
    size: Option<u64>,
    model: Option<String>,
    tran: Option<String>,
    #[serde(rename = "type")]
    devtype: String,
    rm: Option<bool>,
    ro: Option<bool>,
}

// r[impl installer.tui.disk-detection+3]
pub fn detect_block_devices() -> Result<Vec<BlockDevice>> {
    let output = Command::new("lsblk")
        .args([
            "--json",
            "--bytes",
            "--output",
            "NAME,SIZE,MODEL,TRAN,TYPE,RM,RO",
            "--nodeps",
        ])
        .output()
        .context("running lsblk")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("lsblk failed: {stderr}");
    }

    let parsed: LsblkOutput =
        serde_json::from_slice(&output.stdout).context("parsing lsblk JSON output")?;

    let devices = parsed
        .blockdevices
        .into_iter()
        .filter(|d| d.devtype == "disk")
        .filter(|d| d.ro != Some(true))
        .map(|d| {
            let size_bytes = d.size.unwrap_or(0);
            let transport = d
                .tran
                .as_deref()
                .map(TransportType::from_tran)
                .unwrap_or(TransportType::Unknown);
            BlockDevice {
                path: PathBuf::from(format!("/dev/{}", d.name)),
                size_bytes,
                model: d.model.unwrap_or_default().trim().to_string(),
                transport,
                removable: d.rm.unwrap_or(false),
            }
        })
        .collect();

    Ok(devices)
}

/// Resolve a `DiskSelector` against the list of detected block devices.
pub fn resolve_disk<'a>(
    selector: &DiskSelector,
    devices: &'a [BlockDevice],
    boot_device: Option<&PathBuf>,
) -> Result<&'a BlockDevice> {
    let eligible: Vec<&BlockDevice> = devices
        .iter()
        .filter(|d| boot_device.is_none_or(|bd| d.path != *bd))
        .collect();

    if eligible.is_empty() {
        bail!("no eligible target disks found (all devices may be the boot media)");
    }

    match selector {
        DiskSelector::Path(path) => eligible
            .into_iter()
            .find(|d| d.path == *path)
            .with_context(|| format!("disk {} not found among detected devices", path.display())),

        DiskSelector::Strategy(strategy) => resolve_strategy(*strategy, &eligible),
    }
}

fn resolve_strategy<'a>(
    strategy: DiskStrategy,
    devices: &[&'a BlockDevice],
) -> Result<&'a BlockDevice> {
    match strategy {
        DiskStrategy::LargestSsd => {
            let ssds: Vec<&&BlockDevice> = devices.iter().filter(|d| d.is_ssd()).collect();
            if ssds.is_empty() {
                bail!("no SSDs found for largest-ssd strategy");
            }
            ssds.into_iter()
                .max_by_key(|d| d.size_bytes)
                .copied()
                .context("no SSDs found")
        }
        DiskStrategy::Largest => devices
            .iter()
            .max_by_key(|d| d.size_bytes)
            .copied()
            .context("no devices found"),
        DiskStrategy::Smallest => devices
            .iter()
            .min_by_key(|d| d.size_bytes)
            .copied()
            .context("no devices found"),
    }
}

/// Load fake block devices from a JSON file for dry-run testing.
///
/// The JSON file must contain an array of objects with the same fields as
/// `BlockDevice`: `path`, `model`, `size_bytes`, `transport`, and an
/// optional `removable` boolean (default false).
pub fn load_fake_devices(path: &Path) -> Result<Vec<BlockDevice>> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("reading fake devices file: {}", path.display()))?;
    let devices: Vec<BlockDevice> = serde_json::from_str(&contents)
        .with_context(|| format!("parsing fake devices file: {}", path.display()))?;
    Ok(devices)
}

/// Try to determine which block device we booted from, so we can exclude it
/// as an install target.
pub fn detect_boot_device() -> Option<PathBuf> {
    let output = Command::new("lsblk")
        .args(["--json", "--output", "NAME,MOUNTPOINTS,TYPE", "--tree"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    #[derive(Deserialize)]
    struct Out {
        blockdevices: Vec<Dev>,
    }
    #[derive(Deserialize)]
    struct Dev {
        name: String,
        mountpoints: Option<Vec<Option<String>>>,
        #[serde(rename = "type")]
        devtype: String,
        children: Option<Vec<Dev>>,
    }

    fn has_live_mount(dev: &Dev) -> bool {
        let live_paths = ["/run/live/medium", "/cdrom", "/lib/live/mount/medium"];
        let check_mounts = |mps: &[Option<String>]| {
            mps.iter()
                .any(|m| m.as_deref().is_some_and(|p| live_paths.contains(&p)))
        };

        if dev.mountpoints.as_deref().is_some_and(check_mounts) {
            return true;
        }
        if let Some(children) = &dev.children
            && children.iter().any(has_live_mount)
        {
            return true;
        }
        false
    }

    let parsed: Out = serde_json::from_slice(&output.stdout).ok()?;
    parsed
        .blockdevices
        .iter()
        .find(|d| d.devtype == "disk" && has_live_mount(d))
        .map(|d| PathBuf::from(format!("/dev/{}", d.name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_device(path: &str, size: u64, transport: TransportType) -> BlockDevice {
        BlockDevice {
            path: PathBuf::from(path),
            size_bytes: size,
            model: "Test Disk".into(),
            transport,
            removable: false,
        }
    }

    // r[verify installer.config.schema+2]
    #[test]
    fn resolve_largest_ssd() {
        let devices = vec![
            make_device("/dev/sda", 500_000_000_000, TransportType::Sata),
            make_device("/dev/nvme0n1", 1_000_000_000_000, TransportType::Nvme),
            make_device("/dev/nvme1n1", 500_000_000_000, TransportType::Nvme),
        ];
        let selector = DiskSelector::Strategy(DiskStrategy::LargestSsd);
        let result = resolve_disk(&selector, &devices, None).unwrap();
        assert_eq!(result.path, PathBuf::from("/dev/nvme0n1"));
    }

    // r[verify installer.config.schema+2]
    #[test]
    fn resolve_largest() {
        let devices = vec![
            make_device("/dev/sda", 2_000_000_000_000, TransportType::Sata),
            make_device("/dev/nvme0n1", 1_000_000_000_000, TransportType::Nvme),
        ];
        let selector = DiskSelector::Strategy(DiskStrategy::Largest);
        let result = resolve_disk(&selector, &devices, None).unwrap();
        assert_eq!(result.path, PathBuf::from("/dev/sda"));
    }

    // r[verify installer.config.schema+2]
    #[test]
    fn resolve_smallest() {
        let devices = vec![
            make_device("/dev/sda", 500_000_000_000, TransportType::Sata),
            make_device("/dev/sdb", 100_000_000_000, TransportType::Sata),
        ];
        let selector = DiskSelector::Strategy(DiskStrategy::Smallest);
        let result = resolve_disk(&selector, &devices, None).unwrap();
        assert_eq!(result.path, PathBuf::from("/dev/sdb"));
    }

    // r[verify installer.config.schema+2]
    #[test]
    fn resolve_by_path() {
        let devices = vec![
            make_device("/dev/sda", 500_000_000_000, TransportType::Sata),
            make_device("/dev/sdb", 100_000_000_000, TransportType::Sata),
        ];
        let selector = DiskSelector::Path(PathBuf::from("/dev/sdb"));
        let result = resolve_disk(&selector, &devices, None).unwrap();
        assert_eq!(result.path, PathBuf::from("/dev/sdb"));
    }

    // r[verify installer.config.schema+2]
    #[test]
    fn resolve_by_path_not_found() {
        let devices = vec![make_device(
            "/dev/sda",
            500_000_000_000,
            TransportType::Sata,
        )];
        let selector = DiskSelector::Path(PathBuf::from("/dev/nvme0n1"));
        assert!(resolve_disk(&selector, &devices, None).is_err());
    }

    // r[verify installer.tui.disk-detection+3]
    #[test]
    fn resolve_excludes_boot_device() {
        let devices = vec![
            make_device("/dev/sda", 500_000_000_000, TransportType::Sata),
            make_device("/dev/sdb", 1_000_000_000_000, TransportType::Sata),
        ];
        let boot = PathBuf::from("/dev/sda");
        let selector = DiskSelector::Strategy(DiskStrategy::Largest);
        let result = resolve_disk(&selector, &devices, Some(&boot)).unwrap();
        assert_eq!(result.path, PathBuf::from("/dev/sdb"));
    }

    // r[verify installer.config.schema+2]
    #[test]
    fn resolve_largest_ssd_no_ssds() {
        let devices = vec![make_device(
            "/dev/sda",
            500_000_000_000,
            TransportType::Sata,
        )];
        let selector = DiskSelector::Strategy(DiskStrategy::LargestSsd);
        assert!(resolve_disk(&selector, &devices, None).is_err());
    }

    // r[verify installer.tui.disk-detection+3]
    #[test]
    fn resolve_no_eligible_devices() {
        let devices = vec![make_device(
            "/dev/sda",
            500_000_000_000,
            TransportType::Sata,
        )];
        let boot = PathBuf::from("/dev/sda");
        let selector = DiskSelector::Strategy(DiskStrategy::Largest);
        assert!(resolve_disk(&selector, &devices, Some(&boot)).is_err());
    }

    // r[verify installer.tui.disk-detection+3]
    #[test]
    fn format_bytes_display() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1_073_741_824), "1.0 GiB");
        assert_eq!(format_bytes(1_099_511_627_776), "1.0 TiB");
        assert_eq!(format_bytes(500_107_862_016), "465.8 GiB");
    }

    // r[verify installer.dryrun.devices]
    #[test]
    fn load_fake_devices_from_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.json");
        std::fs::write(
            &path,
            r#"[
                {
                    "path": "/dev/sda",
                    "size_bytes": 500000000000,
                    "model": "Fake SSD",
                    "transport": "NVMe",
                    "removable": false
                },
                {
                    "path": "/dev/sdb",
                    "size_bytes": 1000000000000,
                    "model": "Fake HDD",
                    "transport": "SATA"
                }
            ]"#,
        )
        .unwrap();

        let devices = load_fake_devices(&path).unwrap();
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].path, PathBuf::from("/dev/sda"));
        assert_eq!(devices[0].model, "Fake SSD");
        assert_eq!(devices[0].transport, TransportType::Nvme);
        assert!(!devices[0].removable);
        assert_eq!(devices[1].path, PathBuf::from("/dev/sdb"));
        assert_eq!(devices[1].transport, TransportType::Sata);
        assert!(!devices[1].removable);
    }

    // r[verify installer.dryrun.devices]
    #[test]
    fn load_fake_devices_invalid_json() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.json");
        std::fs::write(&path, "not json").unwrap();
        assert!(load_fake_devices(&path).is_err());
    }

    // r[verify installer.dryrun.devices]
    #[test]
    fn load_fake_devices_nonexistent() {
        assert!(load_fake_devices(Path::new("/nonexistent/devices.json")).is_err());
    }

    // r[verify installer.dryrun.devices]
    #[test]
    fn block_device_roundtrip_serde() {
        let dev = make_device("/dev/nvme0n1", 1_000_000_000_000, TransportType::Nvme);
        let json = serde_json::to_string(&dev).unwrap();
        let parsed: BlockDevice = serde_json::from_str(&json).unwrap();
        assert_eq!(dev, parsed);
    }

    // r[verify installer.tui.disk-detection+3]
    #[test]
    fn transport_display() {
        assert_eq!(TransportType::Nvme.to_string(), "NVMe");
        assert_eq!(TransportType::Sata.to_string(), "SATA");
        assert_eq!(TransportType::Usb.to_string(), "USB");
        assert_eq!(TransportType::Virtio.to_string(), "virtio");
        assert_eq!(TransportType::Scsi.to_string(), "SCSI");
        assert_eq!(TransportType::Unknown.to_string(), "unknown");
    }

    // r[verify installer.tui.disk-detection+3]
    #[test]
    fn transport_from_tran_strings() {
        assert_eq!(TransportType::from_tran("nvme"), TransportType::Nvme);
        assert_eq!(TransportType::from_tran("sata"), TransportType::Sata);
        assert_eq!(TransportType::from_tran("ata"), TransportType::Sata);
        assert_eq!(TransportType::from_tran("usb"), TransportType::Usb);
        assert_eq!(TransportType::from_tran("virtio"), TransportType::Virtio);
        assert_eq!(TransportType::from_tran("sas"), TransportType::Scsi);
        assert_eq!(TransportType::from_tran("blah"), TransportType::Unknown);
    }

    // r[verify installer.tui.disk-detection+3]
    #[test]
    fn ssd_detection() {
        assert!(TransportType::Nvme.is_ssd());
        assert!(TransportType::Virtio.is_ssd());
        assert!(!TransportType::Sata.is_ssd());
        assert!(!TransportType::Usb.is_ssd());
    }
}
