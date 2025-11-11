#!/usr/bin/env node

const fs = require("fs");
const path = require("path");

// Read the migration script
const migrateScript = fs.readFileSync(
  path.join(__dirname, "migrate-to-btrfs.sh"),
  "utf8",
);

// Read Tailscale first-boot script and service
const tailscaleFirstBootScript = fs.readFileSync(
  path.join(__dirname, "..", "common", "tailscale-first-boot.sh"),
  "utf8",
);

const tailscaleFirstBootService = fs.readFileSync(
  path.join(__dirname, "..", "common", "tailscale-first-boot.service"),
  "utf8",
);

// Read Tailscale GPG key
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

// Get architecture from command line (defaults to amd64)
const arch = process.argv[3] || "amd64";

// Read packages from packages.txt
const packagesContent = fs.readFileSync(
  path.join(__dirname, "..", "common", "packages.txt"),
  "utf8",
);
const packages = packagesContent
  .split("\n")
  .map((line) => line.trim())
  .filter((line) => line && !line.startsWith("#"))
  .filter((line) => {
    // Filter out GRUB packages for other architectures
    if (arch === "amd64" && line === "grub-efi-arm64") return false;
    if (arch === "arm64" && line === "grub-efi-amd64") return false;
    if (arch === "arm64" && line === "grub-pc") return false;
    return true;
  });

// Read firewall setup script
const firewallSetupScript = fs.readFileSync(
  path.join(__dirname, "..", "common", "setup-firewall.sh"),
  "utf8",
);

// Generate the autoinstall configuration as a plain object
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

    identity: {
      hostname: "ubuntu-server",
      username: "ubuntu",
      password:
        "$y$j9T$C9MpXNEcjdAu20JYDtSNy.$XbgglEtgFBA5s5/k865cGsPGOVG1Cdrk.OGZzH5fDKC",
      // forest-bridge-window-laptop-garden-summer
    },

    ssh: {
      "install-server": true,
      "allow-pw": false,
    },

    storage: {
      layout: {
        name: "custom",
        config: [
          {
            type: "disk",
            id: "disk0",
            ptable: "gpt",
            grub_device: true,
          },
          {
            type: "partition",
            id: "efi",
            device: "disk0",
            size: "512M",
            flag: "boot",
          },
          {
            type: "partition",
            id: "boot",
            device: "disk0",
            size: "1G",
          },
          {
            type: "partition",
            id: "staging",
            device: "disk0",
            size: "4G",
          },
          {
            type: "partition",
            id: "root",
            device: "disk0",
            size: -1,
          },
          {
            type: "format",
            id: "efi-fmt",
            volume: "efi",
            fstype: "fat32",
          },
          {
            type: "format",
            id: "boot-fmt",
            volume: "boot",
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
            id: "boot-mnt",
            device: "boot-fmt",
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
    },

    packages: packages,

    "user-data": {
      disable_root: true,
      package_update: true,
      package_upgrade: true,
    },

    "late-commands": [
      `cat > /target/tmp/migrate-to-btrfs.sh << 'EOFMIGRATE'\n${migrateScript}\nEOFMIGRATE`,
      `cat > /target/tmp/setup-firewall.sh << 'EOFFIREWALL'\n${firewallSetupScript}\nEOFFIREWALL`,
      "cp /cdrom/pool/extras/tailscale.deb /target/tmp/tailscale.deb || true",
      `base64 -d > /target/tmp/tailscale-apt.gpg << 'EOFGPG'\n${tailscaleGpgKey.toString("base64")}\nEOFGPG`,
      `cat > /target/tmp/setup-tailscale.sh << 'EOFTAILSCALE'\n${fs.readFileSync(path.join(__dirname, "..", "common", "setup-tailscale.sh"), "utf8")}\nEOFTAILSCALE`,
      `cat > /target/etc/systemd/system/tailscale-first-boot.service << 'EOFTSSERVICE'\n${tailscaleFirstBootService}\nEOFTSSERVICE`,
      `cat > /target/usr/local/bin/tailscale-first-boot << 'EOFTSBOOT'\n${tailscaleFirstBootScript}\nEOFTSBOOT`,
      "chmod +x /target/usr/local/bin/tailscale-first-boot",
      "curtin in-target -- bash /tmp/migrate-to-btrfs.sh",
      "curtin in-target -- bash /tmp/setup-firewall.sh",
      "curtin in-target -- bash /tmp/setup-tailscale.sh",
      "curtin in-target -- systemctl enable tailscale-first-boot.service",
      "curtin in-target -- systemctl enable ssh",
    ],
  },
};

// Convert to compact JSON (which is valid YAML)
const output = "#cloud-config\n" + JSON.stringify(config.autoinstall);

// Write to stdout or file
if (process.argv[2]) {
  fs.writeFileSync(process.argv[2], output + "\n");
  console.error(`Generated ${process.argv[2]} for ${arch}`);
} else {
  console.log(output);
}
