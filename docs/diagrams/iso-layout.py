"""Live ISO on-disk layout and the verity-protected boot path.

Shows the hybrid ISO + GPT structure (ISO9660 plus appended FAT and squashfs
partitions) and how dm-verity protects both the live rootfs and the partition
images that the installer writes onto the target disk.

Render with: uv run python iso-layout.py
"""

from diagrams import Cluster, Diagram, Edge
from diagrams.generic.compute import Rack
from diagrams.generic.storage import Storage
from diagrams.onprem.security import Vault
from diagrams.programming.flowchart import Action, Database, Document, MultipleDocuments

graph_attr = {
    "fontsize": "18",
    "labelloc": "t",
    "rankdir": "TB",
    "splines": "spline",
    "nodesep": "0.4",
    "ranksep": "0.9",
}

with Diagram(
    "Live ISO: layout and dm-verity boot path",
    show=False,
    filename="iso-layout",
    outformat="png",
    graph_attr=graph_attr,
):
    iso_file = Storage("bes-installer.iso\n(hybrid: ISO9660 +\nGPT, EFI El Torito)")

    with Cluster("ISO9660 contents"):
        boot = Document("/boot/grub/\ngrub.cfg\n(carries verity\nroot hashes)")
        kernel = Document("/live/vmlinuz +\ninitrd.img\n(live-boot,\nveritysetup,\ncryptsetup)")
        squashfs = Database("/live/filesystem.squashfs\n[squashfs | hash | pad | trailer]")

    with Cluster("Appended GPT partitions"):
        besimages = Database("BESIMAGES (squashfs+verity)\nPARTUUID ac9457d6-...\ncontains efi.img,\nxboot.img, root.img,\npartitions.json")
        besconf = Storage("BESCONF (FAT32, rw on USB)\nPARTUUID e2bac42b-...\nbes-install.toml +\nrecovery-keys.txt +\ninstaller-failed.log")

    iso_file >> [boot, kernel, squashfs]
    iso_file >> [besimages, besconf]

    with Cluster("Live boot"):
        firmware = Rack("UEFI firmware /\nrpi-imager USB /\nVM optical drive")
        grub = Action("GRUB EFI\n(BOOTX64.EFI)")
        initramfs = Action("initramfs premount:\nveritysetup open\nfilesystem.squashfs")
        live_root = Action("/dev/mapper/\nlive-verity\nmounted as /")
        firmware >> grub >> kernel
        kernel >> initramfs
        boot >> Edge(label="live.verity.roothash=") >> initramfs
        squashfs >> initramfs
        initramfs >> live_root

    with Cluster("Installer pre-write check"):
        installer = Action("bes-installer\n(running in\nlive rootfs)")
        verity_open = Action("veritysetup open\nimages partition\n(uses --hash-offset\nfrom 8-byte trailer)")
        scan = Action("Sequential read of\nevery partition image\n(integrity check)")
        roothash = Vault("images.verity.roothash=\n(in GRUB cmdline)")
        live_root >> installer
        besimages >> verity_open
        roothash >> verity_open
        verity_open >> scan
        installer >> verity_open

    with Cluster("Install-time inputs"):
        toml = Document("bes-install.toml\n(optional config)")
        besconf >> Edge(label="mount /run/besconf,\nrw remount probe") >> toml
        toml >> installer

    out = MultipleDocuments("recovery-keys.txt\ninstaller-failed.log")
    installer >> Edge(label="writes back when\nBESCONF is rw", style="dashed") >> out
    out >> besconf
