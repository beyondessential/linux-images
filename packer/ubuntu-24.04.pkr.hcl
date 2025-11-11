packer {
  required_version = ">= 1.9.0"
  required_plugins {
    amazon = {
      version = ">= 1.2.0"
      source  = "github.com/hashicorp/amazon"
    }
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
  default = "24.04"
}

variable "aws_region" {
  type    = string
  default = "ap-southeast-2"
}

variable "disk_size" {
  type    = string
  default = "20G"
}

variable "output_directory" {
  type    = string
  default = "output"
}

locals {
  timestamp = regex_replace(timestamp(), "[- TZ:]", "")

  # Architecture-specific settings
  qemu_arch    = var.architecture == "amd64" ? "x86_64" : "aarch64"
  qemu_machine = var.architecture == "amd64" ? "pc" : "virt"
  qemu_binary  = "qemu-system-${local.qemu_arch}"

  # Ubuntu ISO URLs
  iso_base_url = "https://releases.ubuntu.com/${var.ubuntu_version}"
  iso_filename = var.architecture == "amd64" ? "ubuntu-${var.ubuntu_version}-live-server-amd64.iso" : "ubuntu-${var.ubuntu_version}-live-server-arm64.iso"
  iso_url      = "${local.iso_base_url}/${local.iso_filename}"

  # AWS-specific settings
  aws_arch          = var.architecture == "amd64" ? "x86_64" : "arm64"
  aws_instance_type = var.architecture == "amd64" ? "t3.small" : "t4g.small"

  # Output naming
  image_name = "ubuntu-${var.ubuntu_version}-${var.architecture}-${local.timestamp}"
}

# QEMU builder for bare metal images
# Note: Uses Packer's built-in HTTP server to serve autoinstall config during boot
# For standalone ISOs, use autoinstall/remaster-iso.sh to embed config in ISO
source "qemu" "bare-metal" {
  vm_name          = "${local.image_name}.raw"
  output_directory = "${var.output_directory}/bare-metal-${var.architecture}"

  # ISO configuration
  iso_url      = local.iso_url
  iso_checksum = "file:${local.iso_base_url}/SHA256SUMS"

  # VM configuration
  cpus        = 2
  memory      = 2048
  disk_size   = var.disk_size
  format      = "raw"
  accelerator = "kvm"

  # QEMU settings
  qemu_binary    = local.qemu_binary
  machine_type   = local.qemu_machine
  net_device     = "virtio-net"
  disk_interface = "virtio"

  # Boot configuration
  boot_wait = "5s"
  boot_command = [
    "<esc><wait>",
    "e<wait>",
    "<down><down><down><end>",
    " autoinstall ds=nocloud-net\\;s=http://{{ .HTTPIP }}:{{ .HTTPPort }}/",
    "<f10><wait>"
  ]

  # HTTP server for autoinstall
  http_directory = "../autoinstall"

  # SSH configuration
  ssh_username           = "ubuntu"
  ssh_password           = "ubuntu"
  ssh_timeout            = "30m"
  ssh_handshake_attempts = 50

  # Headless mode
  headless = true

  # Shutdown
  shutdown_command = "echo 'ubuntu' | sudo -S shutdown -P now"
}

# AWS AMI builder
source "amazon-ebs" "aws" {
  ami_name      = local.image_name
  instance_type = local.aws_instance_type
  region        = var.aws_region

  # Source AMI filter
  source_ami_filter {
    filters = {
      name                = "ubuntu/images/hvm-ssd*/ubuntu-noble-${var.ubuntu_version}-${local.aws_arch}-server-*"
      root-device-type    = "ebs"
      virtualization-type = "hvm"
      architecture        = local.aws_arch
    }
    most_recent = true
    owners      = ["099720109477"] # Canonical
  }

  # SSH configuration
  ssh_username = "ubuntu"

  # EBS configuration
  launch_block_device_mappings {
    device_name           = "/dev/sda1"
    volume_size           = 20
    volume_type           = "gp3"
    delete_on_termination = true
  }

  # AMI configuration
  ami_virtualization_type = "hvm"
  ena_support             = true
  sriov_support           = true

  tags = {
    Name         = local.image_name
    OS           = "Ubuntu"
    Version      = var.ubuntu_version
    Architecture = var.architecture
    BuildTime    = local.timestamp
    Builder      = "Packer"
  }
}

build {
  name = "ubuntu-custom"

  sources = [
    "source.qemu.bare-metal",
    "source.amazon-ebs.aws"
  ]

  # Upload common scripts
  provisioner "file" {
    source      = "../common/setup-btrfs.sh"
    destination = "/tmp/setup-btrfs.sh"
  }

  provisioner "file" {
    source      = "../common/setup-swap.sh"
    destination = "/tmp/setup-swap.sh"
  }

  provisioner "file" {
    source      = "../common/packages.txt"
    destination = "/tmp/packages.txt"
  }

  provisioner "file" {
    source      = "../common/setup-tailscale.sh"
    destination = "/tmp/setup-tailscale.sh"
  }

  provisioner "file" {
    source      = "../../ansible/roles/tailscale/files/apt.gpg"
    destination = "/tmp/tailscale-apt.gpg"
  }

  # Make scripts executable
  provisioner "shell" {
    inline = [
      "chmod +x /tmp/setup-btrfs.sh",
      "chmod +x /tmp/setup-swap.sh",
      "chmod +x /tmp/setup-tailscale.sh"
    ]
  }

  # Common provisioning
  provisioner "shell" {
    script = "scripts/provision-common.sh"
  }

  # AWS-specific provisioning
  provisioner "shell" {
    script = "scripts/provision-aws.sh"
    only   = ["amazon-ebs.aws"]
  }

  # Cleanup
  provisioner "shell" {
    inline = [
      "sudo apt-get clean",
      "sudo rm -rf /tmp/*",
      "sudo rm -rf /var/tmp/*",
      "sudo rm -f /root/.bash_history",
      "sudo rm -f /home/ubuntu/.bash_history",
      "sudo cloud-init clean --logs"
    ]
  }

  # Post-processing for bare metal images
  post-processor "checksum" {
    checksum_types = ["sha256"]
    output         = "${var.output_directory}/bare-metal-${var.architecture}/${local.image_name}.{{.ChecksumType}}"
    only           = ["qemu.bare-metal"]
  }

  post-processor "compress" {
    output = "${var.output_directory}/bare-metal-${var.architecture}/${local.image_name}.raw.zst"
    only   = ["qemu.bare-metal"]
  }
}
