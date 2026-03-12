# BES Linux Images & ISO installer

AKA "Tamanu Linux"

## Where to get them

<https://github.com/beyondessential/linux-images/releases/latest>

## Which to use

| Variant | Purpose |
|---------|---------|
| AMI | BES Cloud (AWS) |
| cloud image | Other clouds or on-premise virtualisation |
| metal image | Bare metal, direct-to-disk |
| ISO | Bare metal, from USB |

(Prefer to use USB3 drives if you can, USB2 will be very slow.)

| Image Format | Use case |
|--------|----------|
| raw | Write directly to server disk |
| vmdk | VMware / vSphere |
| qcow2 | KVM / libvirt / Proxmox |

The raw images are Zstd-compressed.

## What to configure

We have two separate guides:
- [direct-to-disk images](./docs/GUIDE-IMAGES.md)
- [ISO/USB installer](./docs/GUIDE-INSTALLER.md)

## If things go wrong

While we do our best, we don't have as extensive a testing regimen and hardware availability as upstream Ubuntu Server images and installers.
If you run into issues, you can perform installs using the Ubuntu Server ISO following the "fallback" process in this guide: <https://beyond-essential.slab.com/posts/external-on-premise-tamanu-linux-server-bring-up-onqd6s79>.
