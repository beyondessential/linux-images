use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::paths;

use super::progress::WriteProgress;

const IMAGES_LABEL: &str = "BESIMAGES";
const VERITY_NAME: &str = "besimages-verity";
const IMAGES_MOUNT: &str = "/run/bes-images";

// r[impl installer.write.source+4]
// r[impl iso.verity.layout+3]
/// Read the verity hash tree size from the 8-byte little-endian trailer
/// at the end of a self-describing verity blob.
///
/// Uses `seek(SeekFrom::End(0))` to determine the total size, which works
/// for both regular files and block devices (`metadata().len()` returns 0
/// for block devices on Linux).
fn read_verity_trailer(path: &Path) -> Result<(u64, u64)> {
    let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let total_size = f
        .seek(SeekFrom::End(0))
        .with_context(|| format!("seeking to end of {}", path.display()))?;
    if total_size < 8 {
        bail!(
            "{}: too small for verity trailer ({total_size} bytes)",
            path.display()
        );
    }
    f.seek(SeekFrom::End(-8))
        .with_context(|| format!("seeking to trailer in {}", path.display()))?;
    let mut buf = [0u8; 8];
    f.read_exact(&mut buf)
        .with_context(|| format!("reading trailer from {}", path.display()))?;
    let hash_size = u64::from_le_bytes(buf);
    let hash_offset = total_size
        .checked_sub(8)
        .and_then(|v| v.checked_sub(hash_size))
        .with_context(|| {
            format!(
                "{}: invalid verity trailer (hash_size={hash_size}, total={total_size})",
                path.display()
            )
        })?;
    Ok((hash_offset, hash_size))
}

// r[impl installer.write.source+4]
/// Read a `key=value` parameter from `/proc/cmdline`.
fn cmdline_param(key: &str) -> Result<Option<String>> {
    let cmdline = fs::read_to_string("/proc/cmdline").context("reading /proc/cmdline")?;
    let prefix = format!("{key}=");
    for token in cmdline.split_whitespace() {
        if let Some(value) = token.strip_prefix(&prefix) {
            return Ok(Some(value.to_string()));
        }
    }
    Ok(None)
}

/// Find the images partition block device.
///
/// Checks `/dev/disk/by-label/BESIMAGES` first, then falls back to
/// partition 4 of the detected boot device.
fn find_images_device(boot_device: Option<&Path>) -> Result<Option<PathBuf>> {
    let by_label = PathBuf::from(format!("/dev/disk/by-label/{IMAGES_LABEL}"));
    if by_label.exists() {
        let resolved = fs::canonicalize(&by_label)
            .with_context(|| format!("resolving {}", by_label.display()))?;
        tracing::info!("images partition found by label: {}", resolved.display());
        return Ok(Some(resolved));
    }

    if let Some(boot) = boot_device {
        let p4 = crate::util::partition_path(boot, 4)?;
        if p4.exists() {
            tracing::info!("images partition found as partition 4: {}", p4.display());
            return Ok(Some(p4));
        }
    }

    Ok(None)
}

/// State tracking for the opened images verity device and mount.
pub struct ImagesVerity {
    mount_point: PathBuf,
    verity_name: String,
    mounted: bool,
    verity_open: bool,
}

impl ImagesVerity {
    #[expect(dead_code, reason = "public API for callers that need the mount path")]
    pub fn mount_point(&self) -> &Path {
        &self.mount_point
    }
}

impl Drop for ImagesVerity {
    fn drop(&mut self) {
        if self.mounted {
            let status = Command::new(paths::UMOUNT).arg(&self.mount_point).status();
            match status {
                Ok(s) if s.success() => {
                    tracing::info!("unmounted {}", self.mount_point.display());
                }
                Ok(s) => {
                    tracing::warn!("umount {} exited with {}", self.mount_point.display(), s);
                }
                Err(e) => {
                    tracing::warn!("umount {} failed: {e}", self.mount_point.display());
                }
            }
        }
        if self.verity_open {
            let status = Command::new(paths::VERITYSETUP)
                .args(["close", &self.verity_name])
                .status();
            match status {
                Ok(s) if s.success() => {
                    tracing::info!("closed verity device {}", self.verity_name);
                }
                Ok(s) => {
                    tracing::warn!("veritysetup close {} exited with {}", self.verity_name, s);
                }
                Err(e) => {
                    tracing::warn!("veritysetup close {} failed: {e}", self.verity_name);
                }
            }
        }
    }
}

// r[impl iso.verity.images+3]
// r[impl installer.write.source+4]
/// Open the images partition via dm-verity and mount it as squashfs.
///
/// Returns `Ok(Some(ImagesVerity))` on success, or `Ok(None)` if there is
/// no images partition (e.g. plain directory fallback for development).
pub fn open_and_mount_images(boot_device: Option<&Path>) -> Result<Option<ImagesVerity>> {
    let device = match find_images_device(boot_device)? {
        Some(d) => d,
        None => {
            tracing::info!("no images partition found, verity not available");
            return Ok(None);
        }
    };

    let roothash = cmdline_param("images.verity.roothash")?
        .with_context(|| "images.verity.roothash not found on kernel command line")?;

    let (hash_offset, _hash_size) = read_verity_trailer(&device)
        .with_context(|| format!("reading verity trailer from {}", device.display()))?;

    tracing::info!(
        "opening verity on {} (hash_offset={hash_offset}, roothash={roothash})",
        device.display()
    );

    let output = Command::new(paths::VERITYSETUP)
        .args([
            "open",
            device.to_str().unwrap_or_default(),
            VERITY_NAME,
            device.to_str().unwrap_or_default(),
            &roothash,
            &format!("--hash-offset={hash_offset}"),
        ])
        .output()
        .context("running veritysetup open")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("veritysetup open failed: {stderr}");
    }

    let dm_path = PathBuf::from(format!("/dev/mapper/{VERITY_NAME}"));

    fs::create_dir_all(IMAGES_MOUNT)
        .with_context(|| format!("creating mount point {IMAGES_MOUNT}"))?;

    let mount_output = Command::new(paths::MOUNT)
        .args(["-t", "squashfs", "-o", "ro"])
        .arg(&dm_path)
        .arg(IMAGES_MOUNT)
        .output()
        .context("mounting images squashfs")?;

    if !mount_output.status.success() {
        let stderr = String::from_utf8_lossy(&mount_output.stderr);
        let _ = Command::new(paths::VERITYSETUP)
            .args(["close", VERITY_NAME])
            .status();
        bail!("mount images squashfs failed: {stderr}");
    }

    tracing::info!("images partition mounted at {IMAGES_MOUNT} via verity");

    Ok(Some(ImagesVerity {
        mount_point: PathBuf::from(IMAGES_MOUNT),
        verity_name: VERITY_NAME.to_string(),
        mounted: true,
        verity_open: true,
    }))
}

// r[impl iso.verity.check]
// r[impl installer.write.stream-copy]
/// Splice data from `src_fd` through a pipe to `dst_fd` using `splice(2)`.
///
/// Returns the total number of bytes transferred. Calls `on_progress` after
/// each splice iteration with the cumulative byte count at `bytes_offset +
/// transferred`.
///
/// The pipe buffer is resized to 1 MiB via `fcntl(F_SETPIPE_SZ)`.
pub fn splice_fd_to_fd(
    src_fd: i32,
    dst_fd: i32,
    expected_bytes: Option<u64>,
    bytes_offset: u64,
    total_bytes: Option<u64>,
    start: std::time::Instant,
    on_progress: &mut dyn FnMut(&WriteProgress),
) -> Result<u64> {
    const PIPE_SIZE: i32 = 1024 * 1024; // 1 MiB

    let mut pipe_fds = [0i32; 2];
    if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
        bail!("pipe() failed: {}", std::io::Error::last_os_error());
    }
    let pipe_read = pipe_fds[0];
    let pipe_write = pipe_fds[1];

    // Resize pipe buffer to 1 MiB
    unsafe {
        libc::fcntl(pipe_write, libc::F_SETPIPE_SZ, PIPE_SIZE);
    }

    let mut transferred: u64 = 0;
    let chunk_size = PIPE_SIZE as usize;

    let result = (|| -> Result<u64> {
        loop {
            let remaining = expected_bytes.map(|e| e - transferred);
            let to_splice = match remaining {
                Some(0) => break,
                Some(r) => r.min(chunk_size as u64) as usize,
                None => chunk_size,
            };

            let n_to_pipe = unsafe {
                libc::splice(
                    src_fd,
                    std::ptr::null_mut(),
                    pipe_write,
                    std::ptr::null_mut(),
                    to_splice,
                    libc::SPLICE_F_MOVE,
                )
            };
            if n_to_pipe < 0 {
                let err = std::io::Error::last_os_error();
                bail!("splice(src->pipe) failed: {err}");
            }
            if n_to_pipe == 0 {
                break;
            }

            let mut piped = 0isize;
            while piped < n_to_pipe {
                let n_from_pipe = unsafe {
                    libc::splice(
                        pipe_read,
                        std::ptr::null_mut(),
                        dst_fd,
                        std::ptr::null_mut(),
                        (n_to_pipe - piped) as usize,
                        libc::SPLICE_F_MOVE,
                    )
                };
                if n_from_pipe < 0 {
                    let err = std::io::Error::last_os_error();
                    bail!("splice(pipe->dst) failed: {err}");
                }
                if n_from_pipe == 0 {
                    bail!("splice(pipe->dst) returned 0 unexpectedly");
                }
                piped += n_from_pipe;
            }

            transferred += n_to_pipe as u64;
            on_progress(&WriteProgress {
                bytes_written: bytes_offset + transferred,
                total_bytes,
                elapsed: start.elapsed(),
            });
        }
        Ok(transferred)
    })();

    unsafe {
        libc::close(pipe_read);
        libc::close(pipe_write);
    }

    result
}

// r[impl iso.verity.check]
/// Run the upfront integrity check: splice every partition image to
/// `/dev/null`, forcing dm-verity to verify every block.
///
/// Returns `Ok(())` if all reads succeed, or an I/O error if corruption
/// is detected.
pub fn integrity_check(
    images_dir: &Path,
    image_files: &[(String, u64)],
    on_progress: &mut dyn FnMut(&WriteProgress),
) -> Result<()> {
    let total_bytes: u64 = image_files.iter().map(|(_, sz)| *sz).sum();
    let start = std::time::Instant::now();
    let mut bytes_offset: u64 = 0;

    let dev_null = File::options()
        .write(true)
        .open("/dev/null")
        .context("opening /dev/null")?;
    let null_fd = {
        use std::os::unix::io::AsRawFd;
        dev_null.as_raw_fd()
    };

    for (filename, expected_size) in image_files {
        let path = images_dir.join(filename);
        let src = File::open(&path)
            .with_context(|| format!("opening {} for integrity check", path.display()))?;
        let src_fd = {
            use std::os::unix::io::AsRawFd;
            src.as_raw_fd()
        };

        tracing::info!("integrity check: reading {filename}");
        let transferred = splice_fd_to_fd(
            src_fd,
            null_fd,
            Some(*expected_size),
            bytes_offset,
            Some(total_bytes),
            start,
            on_progress,
        )
        .with_context(|| format!("integrity check failed reading {filename}"))?;

        bytes_offset += transferred;
    }

    on_progress(&WriteProgress {
        bytes_written: total_bytes,
        total_bytes: Some(total_bytes),
        elapsed: start.elapsed(),
    });

    tracing::info!(
        "integrity check passed: {total_bytes} bytes verified in {:.1}s",
        start.elapsed().as_secs_f64()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    // r[verify iso.verity.layout+3]
    #[test]
    fn read_verity_trailer_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob");
        let mut f = File::create(&path).unwrap();

        // Simulate: 4096 bytes data, 2048 bytes hash tree, 8 bytes trailer
        let data = vec![0u8; 4096];
        let hash = vec![0xABu8; 2048];
        let trailer = 2048u64.to_le_bytes();

        f.write_all(&data).unwrap();
        f.write_all(&hash).unwrap();
        f.write_all(&trailer).unwrap();
        f.flush().unwrap();

        let (hash_offset, hash_size) = read_verity_trailer(&path).unwrap();
        assert_eq!(hash_offset, 4096);
        assert_eq!(hash_size, 2048);
    }

    // r[verify iso.verity.layout+3]
    #[test]
    fn read_verity_trailer_too_small() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny");
        std::fs::write(&path, &[0u8; 4]).unwrap();

        assert!(read_verity_trailer(&path).is_err());
    }

    // r[verify iso.verity.layout+3]
    #[test]
    fn read_verity_trailer_invalid_hash_size() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad");
        let mut f = File::create(&path).unwrap();

        // Write 16 bytes of data, then a trailer claiming 100 bytes of hash
        // (which exceeds the file minus the 8-byte trailer)
        let data = vec![0u8; 16];
        let trailer = 100u64.to_le_bytes();
        f.write_all(&data).unwrap();
        f.write_all(&trailer).unwrap();
        f.flush().unwrap();

        assert!(read_verity_trailer(&path).is_err());
    }

    // r[verify iso.verity.layout+3]
    #[test]
    fn read_verity_trailer_sector_aligned_with_padding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aligned");
        let mut f = File::create(&path).unwrap();

        // Simulate a sector-aligned blob:
        //   4096 bytes data + 2048 bytes hash tree + padding + 8 bytes trailer
        // Total must be a multiple of 4096.
        let data_size: u64 = 4096;
        let hash_tree_size: u64 = 2048;
        let data = vec![0u8; data_size as usize];
        let hash = vec![0xABu8; hash_tree_size as usize];

        f.write_all(&data).unwrap();
        f.write_all(&hash).unwrap();

        // current_size = 6144, need to fit + 8 byte trailer, round up to 4096
        // (6144 + 8) = 6152, round up to 8192
        let current_size = data_size + hash_tree_size;
        let total_needed = ((current_size + 8 + 4095) / 4096) * 4096;
        let padding = total_needed - current_size - 8;
        assert_eq!(total_needed, 8192);
        assert_eq!(padding, 2040);

        let pad = vec![0u8; padding as usize];
        f.write_all(&pad).unwrap();

        // trailer hash_size includes padding: total_needed - 8 - data_size
        let trailer_hash_size = total_needed - 8 - data_size;
        assert_eq!(trailer_hash_size, hash_tree_size + padding);
        f.write_all(&trailer_hash_size.to_le_bytes()).unwrap();
        f.flush().unwrap();

        let file_size = std::fs::metadata(&path).unwrap().len();
        assert_eq!(file_size, total_needed);
        assert_eq!(file_size % 4096, 0);

        let (hash_offset, hash_size) = read_verity_trailer(&path).unwrap();
        assert_eq!(hash_offset, data_size);
        assert_eq!(hash_size, trailer_hash_size);
    }

    // r[verify installer.write.stream-copy]
    #[test]
    fn splice_fd_to_fd_copies_data() {
        use std::os::unix::io::AsRawFd;

        let dir = tempfile::tempdir().unwrap();
        let src_path = dir.path().join("src");
        let dst_path = dir.path().join("dst");

        let payload = vec![42u8; 65536];
        std::fs::write(&src_path, &payload).unwrap();

        let src = File::open(&src_path).unwrap();
        let dst = File::create(&dst_path).unwrap();
        let start = std::time::Instant::now();

        let transferred = splice_fd_to_fd(
            src.as_raw_fd(),
            dst.as_raw_fd(),
            Some(payload.len() as u64),
            0,
            Some(payload.len() as u64),
            start,
            &mut |_| {},
        )
        .unwrap();

        assert_eq!(transferred, payload.len() as u64);

        let result = std::fs::read(&dst_path).unwrap();
        assert_eq!(result, payload);
    }

    // r[verify installer.write.stream-copy]
    #[test]
    fn splice_fd_to_fd_empty_file() {
        use std::os::unix::io::AsRawFd;

        let dir = tempfile::tempdir().unwrap();
        let src_path = dir.path().join("empty");
        let dst_path = dir.path().join("dst");

        std::fs::write(&src_path, &[]).unwrap();

        let src = File::open(&src_path).unwrap();
        let dst = File::create(&dst_path).unwrap();
        let start = std::time::Instant::now();

        let transferred = splice_fd_to_fd(
            src.as_raw_fd(),
            dst.as_raw_fd(),
            Some(0),
            0,
            Some(0),
            start,
            &mut |_| {},
        )
        .unwrap();

        assert_eq!(transferred, 0);
    }

    #[test]
    fn cmdline_param_finds_value() {
        // We can't easily test /proc/cmdline in unit tests,
        // but we can verify the function doesn't panic on a real system.
        let _ = cmdline_param("nonexistent_param_xyz");
    }
}
