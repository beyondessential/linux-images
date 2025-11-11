#!/usr/bin/env node

const fs = require("fs");
const path = require("path");

// Simple YAML generator for autoinstall config
function toYAML(obj, indent = 0) {
  const spaces = "  ".repeat(indent);
  let yaml = "";

  for (const [key, value] of Object.entries(obj)) {
    if (value === null || value === undefined) {
      yaml += `${spaces}${key}: null\n`;
    } else if (typeof value === "boolean") {
      yaml += `${spaces}${key}: ${value}\n`;
    } else if (typeof value === "number") {
      yaml += `${spaces}${key}: ${value}\n`;
    } else if (typeof value === "string") {
      // Escape strings with special characters or newlines
      if (value.includes("\n") || value.includes(":") || value.includes("#")) {
        const escaped = value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
        yaml += `${spaces}${key}: "${escaped}"\n`;
      } else {
        yaml += `${spaces}${key}: ${value}\n`;
      }
    } else if (Array.isArray(value)) {
      yaml += `${spaces}${key}:\n`;
      for (const item of value) {
        if (typeof item === "object" && item !== null) {
          yaml += `${spaces}- \n`;
          yaml +=
            toYAML(item, indent + 1)
              .split("\n")
              .filter((line) => line)
              .map((line) => `  ${line}`)
              .join("\n") + "\n";
        } else if (typeof item === "string") {
          if (item.includes("\n") || item.includes(":") || item.includes("#")) {
            const escaped = item.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
            yaml += `${spaces}- "${escaped}"\n`;
          } else {
            yaml += `${spaces}- ${item}\n`;
          }
        } else {
          yaml += `${spaces}- ${item}\n`;
        }
      }
    } else if (typeof value === "object") {
      yaml += `${spaces}${key}:\n`;
      yaml += toYAML(value, indent + 1);
    }
  }

  return yaml;
}

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

// Read packages from packages.txt
const packagesContent = fs.readFileSync(
  path.join(__dirname, "..", "common", "packages.txt"),
  "utf8",
);
const packages = packagesContent
  .split("\n")
  .map((line) => line.trim())
  .filter((line) => line && !line.startsWith("#"));

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

    interactive: {
      sections: ["network", "identity"],
    },

    locale: "en_US.UTF-8",

    keyboard: {
      layout: "us",
    },

    timezone: "Etc/UTC",

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
      "curtin in-target -- bash /tmp/migrate-to-btrfs.sh",
      `cat > /target/tmp/migrate-to-btrfs.sh << 'EOFMIGRATE'\n${migrateScript}\nEOFMIGRATE`,
      "curtin in-target -- bash /tmp/setup-firewall.sh",
      `cat > /target/tmp/setup-firewall.sh << 'EOFFIREWALL'\n${firewallSetupScript}\nEOFFIREWALL`,
      "cp /cdrom/pool/extras/tailscale.deb /target/tmp/tailscale.deb || true",
      `base64 -d > /target/tmp/tailscale-apt.gpg << 'EOFGPG'\n${tailscaleGpgKey.toString("base64")}\nEOFGPG`,
      "curtin in-target -- bash /tmp/setup-tailscale.sh",
      `cat > /target/tmp/setup-tailscale.sh << 'EOFTAILSCALE'\n${fs.readFileSync(path.join(__dirname, "..", "common", "setup-tailscale.sh"), "utf8")}\nEOFTAILSCALE`,
      `cat > /target/etc/systemd/system/tailscale-first-boot.service << 'EOFTSSERVICE'\n${tailscaleFirstBootService}\nEOFTSSERVICE`,
      `cat > /target/usr/local/bin/tailscale-first-boot << 'EOFTSBOOT'\n${tailscaleFirstBootScript}\nEOFTSBOOT`,
      "chmod +x /target/usr/local/bin/tailscale-first-boot",
      "curtin in-target -- systemctl enable tailscale-first-boot.service",
      "curtin in-target -- systemctl enable ssh",
    ],
  },
};

// Convert to YAML
const output = "#cloud-config\nautoinstall:\n" + toYAML(config.autoinstall, 1);


// Write to stdout or file
if (process.argv[2]) {
  fs.writeFileSync(process.argv[2], output + "\n");
  console.error(`Generated ${process.argv[2]}`);
} else {
  console.log(output);
}
