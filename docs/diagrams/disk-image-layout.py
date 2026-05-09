"""On-disk layout of a deployed image, side by side per variant.

The three build-time variants share a partition skeleton (EFI/firmware,
xboot, root) with BTRFS subvolumes inside; encryption and bootloader
differ. This diagram lays them out in parallel so the differences are
obvious at a glance.

Render with: uv run python disk-image-layout.py
"""

from diagrams import Cluster, Diagram, Edge
from diagrams.generic.storage import Storage
from diagrams.onprem.security import Vault
from diagrams.programming.flowchart import Action, Database, Document, InternalStorage

graph_attr = {
    "fontsize": "18",
    "labelloc": "t",
    "rankdir": "TB",
    "splines": "ortho",
    "nodesep": "0.4",
    "ranksep": "0.7",
}


def variant(label: str, *, encrypted: bool, pi: bool):
    with Cluster(label):
        if pi:
            efi = Storage(
                "firmware (FAT32)\nlabel=firmware\n>=1 GiB\n/boot/firmware\n(config.txt,\nvmlinuz, initrd,\nbcm2712-rpi-5-b.dtb)"
            )
        else:
            efi = Storage(
                "EFI System (FAT32)\nlabel=efi, 512 MiB\nGRUB EFI binary"
            )
        xboot = Storage("xboot (ext4)\nlabel=xboot, 1 GiB\nLinux extended boot")
        if encrypted:
            luks = Vault("LUKS2 root partition\nlabel=root\nrest of disk")
            btrfs_label = "BTRFS on /dev/mapper/root"
        else:
            luks = None
            btrfs_label = "BTRFS root partition"
        with Cluster(btrfs_label):
            sub_root = InternalStorage("@\n-> /")
            sub_pg = InternalStorage("@postgres\n-> /var/lib/\npostgresql")
        if luks is not None:
            xboot >> Edge(style="invis") >> luks
            luks >> Edge(label="cryptsetup\nopen") >> sub_root
        else:
            xboot >> Edge(style="invis") >> sub_root
        return efi, xboot, luks, sub_root, sub_pg


with Diagram(
    "Deployed disk image: GPT layout per variant",
    show=False,
    filename="disk-image-layout",
    outformat="png",
    graph_attr=graph_attr,
):
    cloud = variant("cloud (no LUKS)", encrypted=False, pi=False)
    metal = variant("metal (LUKS, empty passphrase\nuntil installer rotates)", encrypted=True, pi=False)
    pi = variant("pi (arm64, LUKS, no GRUB)", encrypted=True, pi=True)

    with Cluster("Boot path"):
        grub = Action("GRUB +\ninitramfs (dracut)")
        eeprom = Action("Pi 5 EEPROM\nfirmware\n(config.txt +\ncmdline.txt)")
        sysd = Action("systemd-cryptsetup\n(crypttab,\nkeyfile or TPM PCR1)")
        grub >> Edge(label="loads kernel +\ninitramfs") >> sysd
        eeprom >> Edge(label="loads kernel +\ninitramfs") >> sysd

    cloud[0] >> Edge(label="EFI/BOOT/...EFI", style="dashed") >> grub
    metal[0] >> Edge(label="EFI/BOOT/...EFI", style="dashed") >> grub
    pi[0] >> Edge(label="config.txt +\ncmdline.txt", style="dashed") >> eeprom

    sysd >> Edge(label="opens", style="dashed") >> metal[2]
    sysd >> Edge(label="opens", style="dashed") >> pi[2]

    with Cluster("Annotations"):
        notes = Document(
            "compress=zstd:6,\nxxhash, simple\nquotas, block-group-tree.\n"
            "Snapper: 6 hourly +\n10 numbered\non / and /var/lib/postgresql.\n"
            "/etc/bes/image-variant\nrecords runtime mode:\nplain | luks-keyfile |\nluks-tpm."
        )
