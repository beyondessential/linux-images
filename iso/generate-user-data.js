#!/usr/bin/env node

const fs = require("fs");
const path = require("path");

const fixPartitionsScript = fs.readFileSync(
  path.join(__dirname, "fix-partitions.sh"),
  "utf8",
);

const repartitionScript = fs.readFileSync(
  path.join(__dirname, "repartition.sh"),
  "utf8",
);

const remountTargetScript = fs.readFileSync(
  path.join(__dirname, "remountTarget.sh"),
  "utf8",
);

const bootloaderScript = fs.readFileSync(
  path.join(__dirname, "bootloader.sh"),
  "utf8",
);

const luksReencryptService = fs.readFileSync(
  path.join(__dirname, "luks-reencrypt.service"),
  "utf8",
);

const diskGrowthService = fs.readFileSync(
  path.join(__dirname, "grow-root-filesystem.service"),
  "utf8",
);

const diskGrowthScript = fs.readFileSync(
  path.join(__dirname, "grow-root-filesystem"),
  "utf8",
);

const tailscaleUpScript = fs.readFileSync(
  path.join(__dirname, "ts-up.sh"),
  "utf8",
);

const tailscaleGpgKey = fs.readFileSync(
  path.join(
    __dirname,
    "..",
    "..",
    "ansible",
    "roles",
    "tailscale",
    "files",
    "apt.gpg",
  ),
);

const arch = process.argv?.[2] ?? "amd64";

const packagesContent = fs.readFileSync(
  path.join(__dirname, "packages.txt"),
  "utf8",
);
const packages = packagesContent
  .split("\n")
  .map((line) => line.trim())
  .filter((line) => line && !line.startsWith("#"));

const firewallSetupScript = fs.readFileSync(
  path.join(__dirname, "setup-firewall.sh"),
  "utf8",
);

// "bes"
// passwords are not allowed for SSH so this is purely a console-based login thing
const password =
  "$y$j9T$Kh7z7p6FH3zn9r4HAMB1i0$dgY1wDtDbL4do748v9q32AV2qE5kgz0vqW8rUHQFox9";

const config = {
  autoinstall: {
    version: 1,
    source: {
      id: "ubuntu-server-minimal",
    },

    locale: "en_US.UTF-8",
    timezone: "Etc/UTC",
    keyboard: {
      layout: "us",
    },

    ssh: {
      "install-server": true,
      "allow-pw": false,
    },

    storage: {
      version: 2,
      swap: {
        size: 0,
      },
      grub: {
        reorder_uefi: false,
      },
      config: [
        {
          type: "disk",
          id: "disk0",
          ptable: "gpt",
          wipe: "superblock-recursive",
          preserve: false,
          match: [
            {
              size: "largest",
              ssd: true,
            },
            {
              size: "largest",
            },
          ],
        },
        {
          type: "partition",
          id: "efi",
          device: "disk0",
          size: "512M",
          flag: "boot",
          partition_name: "efi",
          partition_type: "c12a7328-f81f-11d2-ba4b-00a0c93ec93b",
          grub_device: true,
          wipe: "superblock",
          preserve: false,
        },
        {
          type: "partition",
          id: "xboot",
          device: "disk0",
          size: "1G",
          partition_name: "boot",
          partition_type: "bc13c2ff-59e6-4262-a352-b275fd6f7172",
          wipe: "superblock",
          preserve: false,
        },
        {
          type: "partition",
          id: "staging",
          device: "disk0",
          size: "4G",
          partition_name: "swap",
          partition_type: "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f",
          wipe: "superblock",
          preserve: false,
        },
        {
          type: "partition",
          id: "root",
          device: "disk0",
          size: -1,
          partition_name: "root",
          partition_type:
            arch == "amd64"
              ? "4f68bce3-e8cd-4db1-96e7-fbcaf984b709"
              : "b921b045-1df0-41c3-af44-4c6f280d3fae",
          wipe: "superblock",
          preserve: false,
        },
        {
          type: "format",
          id: "efi-fmt",
          volume: "efi",
          fstype: "fat32",
        },
        {
          type: "format",
          id: "xboot-fmt",
          volume: "xboot",
          fstype: "ext4",
        },
        {
          type: "format",
          id: "staging-fmt",
          volume: "staging",
          fstype: "ext4",
        },
        {
          type: "mount",
          id: "efi-mnt",
          device: "efi-fmt",
          path: "/boot/efi",
        },
        {
          type: "mount",
          id: "xboot-mnt",
          device: "xboot-fmt",
          path: "/boot",
        },
        {
          type: "mount",
          id: "staging-mnt",
          device: "staging-fmt",
          path: "/",
        },
      ],
    },

    packages: packages,

    "user-data": {
      disable_root: true,
      package_update: true,
      package_upgrade: true,
      create_hostname_file: false, // provided by DHCP
      ssh_pwauth: false,
      users: ["default"],
      chpasswd: {
        users: [
          { name: "root", password },
          { name: "ubuntu", password },
        ],
      },
      runcmd: [
        ["passwd", "--expire", "ubuntu"],
        ["passwd", "--expire", "root"],
      ],
    },

    "late-commands": [
      // helper to wipe the disk if needed (on error)
      `echo "dd if=/dev/zero of=/dev/$(lsblk -ndo PKNAME $(findmnt -no SOURCE /target)) bs=1M status=progress" > wipe-target.sh`,
      // network configuration (in original /target)
      `cat > /target/tmp/setup-firewall.sh << 'EOFFIREWALL'\n${firewallSetupScript}\nEOFFIREWALL`,
      `base64 -d > /target/tmp/tailscale-apt.gpg << 'EOFGPG'\n${tailscaleGpgKey.toString("base64")}\nEOFGPG`,
      `cat > /target/tmp/setup-tailscale.sh << 'EOFTAILSCALE'\n${fs.readFileSync(path.join(__dirname, "setup-tailscale.sh"), "utf8")}\nEOFTAILSCALE`,
      `cat > /target/usr/local/bin/ts-up << 'EOFTSUP'\n${tailscaleUpScript}\nEOFTSUP`,
      "chmod +x /target/usr/local/bin/ts-up",
      "curtin in-target --target=/target -- bash /tmp/setup-firewall.sh",
      "curtin in-target --target=/target -- bash /tmp/setup-tailscale.sh",
      "curtin in-target --target=/target -- systemctl enable ssh",
      // subiquity doesn't set labels and type uuids correctly, so fix that
      `cat > /target/tmp/fix-partitions.sh << 'EOFFIX'\n${fixPartitionsScript}\nEOFFIX`,
      "curtin in-target --target=/target -- bash /tmp/fix-partitions.sh",
      // setup the real root as encrypted btrfs with subvolumes and copy data into it
      `cat > /target/tmp/repartition.sh << 'EOFMIGRATE'\n${repartitionScript}\nEOFMIGRATE`,
      "curtin in-target --target=/target -- bash /tmp/repartition.sh",
      // unmount /target, wipe staging, make it into encrypted swap, mount the real root to /target
      `cat > /tmp/remountTarget.sh << 'EOFMIGRATE'\n${remountTargetScript}\nEOFMIGRATE`,
      "bash /tmp/remountTarget.sh",
      // re-install the bootloader from the real root
      `cat > /target/tmp/bootloader.sh << 'EOFMIGRATE'\n${bootloaderScript}\nEOFMIGRATE`,
      "curtin in-target --target=/target -- bash /tmp/bootloader.sh",
      // setup LUKS key rotation service
      `cat > /target/etc/systemd/system/luks-reencrypt.service << 'EOFLUKSREENCRYPT'\n${luksReencryptService}\nEOFLUKSREENCRYPT`,
      "curtin in-target --target=/target -- systemctl enable luks-reencrypt.service",
      // setup disk growth service and script
      `cat > /target/usr/local/bin/grow-root-filesystem << 'EOFDISKGROWTH'\n${diskGrowthScript}\nEOFDISKGROWTH`,
      "chmod +x /target/usr/local/bin/grow-root-filesystem",
      `cat > /target/etc/systemd/system/grow-root-filesystem.service << 'EOFDISKGROWTHSVC'\n${diskGrowthService}\nEOFDISKGROWTHSVC`,
      "curtin in-target --target=/target -- systemctl enable grow-root-filesystem.service",
    ],

    "error-commands": [
      "tar -czf /target/installer-logs.tar.gz /var/log/installer/",
      "journalctl -b > /target/installer-journal.log",
    ],
  },
};

const output = "#cloud-config\n" + JSON.stringify(config.autoinstall);
console.log(output);
