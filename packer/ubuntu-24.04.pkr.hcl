packer {
  required_version = ">= 1.9.0"
  required_plugins {
    qemu = {
      version = ">= 1.0.0"
      source  = "github.com/hashicorp/qemu"
    }
  }
}

variable "architecture" {
  type        = string
  description = "Target architecture: amd64 or arm64"

  validation {
    condition     = contains(["amd64", "arm64"], var.architecture)
    error_message = "Architecture must be either 'amd64' or 'arm64'."
  }
}

variable "ubuntu_version" {
  type    = string
  default = "24.04.3"
}

variable "disk_size" {
  type    = string
  default = "20G"
}

variable "output_directory" {
  type    = string
  default = "output"
}

variable "custom_iso_path" {
  type        = string
  description = "Path to custom ISO with embedded autoinstall"
  default     = ""
}

locals {
  timestamp = regex_replace(timestamp(), "[- TZ:]", "")

  qemu_arch    = var.architecture == "amd64" ? "x86_64" : "aarch64"
  qemu_machine = var.architecture == "amd64" ? "pc" : "virt"
  qemu_binary  = "qemu-system-${local.qemu_arch}"

  image_name = "ubuntu-${var.ubuntu_version}-${var.architecture}-${local.timestamp}"

  iso_path = var.custom_iso_path != "" ? var.custom_iso_path : "../iso/ubuntu-${var.ubuntu_version}-bes-server-${var.architecture}-*.iso"
}

source "qemu" "bare-metal" {
  vm_name          = "${local.image_name}.raw"
  output_directory = "${var.output_directory}/bare-metal-${var.architecture}"

  iso_url      = local.iso_path
  iso_checksum = "none"

  cpus        = 2
  memory      = 4096
  disk_size   = var.disk_size
  format      = "raw"
  accelerator = "kvm"

  qemu_binary    = local.qemu_binary
  machine_type   = local.qemu_machine
  net_device     = "virtio-net"
  disk_interface = "virtio"

  boot_wait    = "3s"
  boot_command = ["<enter>"]

  ssh_username           = "ubuntu"
  ssh_password           = "bes"
  ssh_timeout            = "60m"
  ssh_handshake_attempts = 100

  headless = true

  shutdown_command = "echo 'bes' | sudo -S shutdown -P now"
}

build {
  name = "ubuntu-bare-metal"

  sources = [
    "source.qemu.bare-metal"
  ]

  provisioner "shell" {
    inline = [
      "echo 'Build complete for ${var.architecture}'",
      "uname -a",
      "df -h",
      "lsblk"
    ]
  }

  provisioner "shell" {
    inline = [
      "sudo apt-get clean",
      "sudo rm -rf /tmp/*",
      "sudo rm -rf /var/tmp/*",
      "sudo rm -f /root/.bash_history",
      "sudo rm -f /home/ubuntu/.bash_history"
    ]
  }

  post-processor "checksum" {
    checksum_types = ["sha256"]
    output         = "${var.output_directory}/bare-metal-${var.architecture}/${local.image_name}.{{.ChecksumType}}"
  }

  post-processor "compress" {
    output            = "${var.output_directory}/bare-metal-${var.architecture}/${local.image_name}.raw.zst"
    compression_level = 9
  }
}
