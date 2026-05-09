"""Installer post-confirmation flow: write -> encrypt -> rebuild boot ->
install-time configuration -> reboot.

Render with: uv run python installer-write.py
"""

from diagrams import Cluster, Diagram, Edge
from diagrams.onprem.security import Vault
from diagrams.programming.flowchart import (
    Action,
    Decision,
    PredefinedProcess,
    StartEnd,
)

graph_attr = {
    "fontsize": "18",
    "labelloc": "t",
    "rankdir": "LR",
    "splines": "spline",
    "nodesep": "0.4",
    "ranksep": "0.7",
}

with Diagram(
    "Installer: write + encryption + finalise",
    show=False,
    filename="installer-write",
    outformat="png",
    graph_attr=graph_attr,
):
    confirmed = StartEnd("Confirmed\n(or auto mode)")
    pass_doc = Vault("recovery\npassphrase")
    confirmed >> Edge(label="generate", style="dashed") >> pass_doc

    with Cluster("Write phase"):
        wipe = PredefinedProcess("Wipe signatures +\nGPT + partitions\n(per partitions.json)")
        verity_check = PredefinedProcess("Upfront integrity check\n(stream every image,\nverity-on-read)")
        luks_format = PredefinedProcess("Format LUKS2 with\nrecovery passphrase\n(if encrypted)")
        write_imgs = PredefinedProcess("Stream copy efi/xboot/root\n(bounded cache)")
        expand = PredefinedProcess("Expand root partition\n+ btrfs resize")
        randomise = PredefinedProcess("Randomise UUIDs\n(tune2fs / btrfstune /\nmlabel)")

    confirmed >> wipe >> verity_check >> luks_format >> write_imgs >> expand >> randomise
    pass_doc >> Edge(style="dashed") >> luks_format

    with Cluster("Encryption setup"):
        enroll = Decision("encryption?")
        tpm_enroll = Action("systemd-cryptenroll\n--tpm2-pcrs=1")
        keyfile_enroll = Action("luksAddKey +\n/etc/luks/keyfile\n(mode 000)")
        crypttab = Action("Write crypttab +\ndracut keyfile\nconfig")

    randomise >> enroll
    enroll >> Edge(label="tpm") >> tpm_enroll >> crypttab
    enroll >> Edge(label="keyfile") >> keyfile_enroll >> crypttab
    enroll >> Edge(label="none", style="dashed") >> crypttab

    rebuild = PredefinedProcess(
        "Rebuild boot config in chroot:\nstrip portable dracut conf,\ndelete old initramfs,\nfstab UUID fixup,\ndracut --force +\nupdate-grub"
    )
    crypttab >> rebuild

    with Cluster("Install-time configuration"):
        host_apply = Action("hostname\n(/etc/hostname,\n/etc/hosts)")
        ts_apply = Action("Tailscale\n(chroot tailscale up\nor /etc/bes/\ntailscale-authkey)")
        ssh_apply = Action("authorized_keys\n(0600,\nubuntu user)")
        pwd_apply = Action("ubuntu password\n(SHA-512 crypt or\npre-hashed)")
        tz_apply = Action("/etc/localtime symlink\n+ /etc/timezone")
        net_apply = Action("netplan YAML for\nstatic / SLAAC /\noffline")
        log_apply = Action("Copy installer log\n-> /var/log/\nbes-installer.log")
        recovery_save = Action("Append recovery\npassphrase to\nBESCONF\n(when writable +\nopt-in)")

    rebuild >> host_apply >> ts_apply >> ssh_apply >> pwd_apply >> tz_apply >> net_apply >> log_apply >> recovery_save
    pass_doc >> Edge(style="dashed") >> recovery_save

    final = StartEnd("Unmount, close LUKS,\nreboot")
    recovery_save >> final
