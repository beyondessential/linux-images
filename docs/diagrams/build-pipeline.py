"""Build pipeline: how source code and external dependencies become artifacts.

Mirrors the structure of `justfile` recipes and the CI matrix in
`.github/workflows/build.yml`. Sub-flows for the disk image, the live ISO,
and the Pi EEPROM SD artifact are each grouped together.

Render with: uv run python build-pipeline.py
"""

from diagrams import Cluster, Diagram, Edge
from diagrams.generic.os import Ubuntu
from diagrams.generic.storage import Storage
from diagrams.onprem.ci import GithubActions
from diagrams.onprem.compute import Server
from diagrams.onprem.container import Docker
from diagrams.onprem.vcs import Git
from diagrams.programming.flowchart import Action, Document, MultipleDocuments, PredefinedProcess
from diagrams.programming.language import Bash, Rust

graph_attr = {
    "fontsize": "18",
    "labelloc": "t",
    "rankdir": "LR",
    "splines": "spline",
    "nodesep": "0.4",
    "ranksep": "1.0",
}

with Diagram(
    "BES Linux Images: build pipeline",
    show=False,
    filename="build-pipeline",
    outformat="png",
    graph_attr=graph_attr,
):
    repo = Git("linux-images\n(this repo)")
    mirror = Ubuntu("Ubuntu archive\n+ bes-tools / kopia /\ntailscale apt repos")
    ci = GithubActions("GitHub Actions\n(amd64 + arm64\nrunners)")
    repo >> ci
    mirror >> Edge(label="debootstrap +\napt install", style="dashed") >> ci

    with Cluster("Disk image build (per arch x variant)"):
        installer_bin = Rust("bes-installer\n(installer/tui)")
        image_build = Bash("image/build.sh\n(debootstrap +\nconfigure.sh)")
        raw = Storage("raw .img")
        post = Action("vmdk + qcow2\nconvert,\nzstd compress")
        image_build >> raw >> post

    with Cluster("Live ISO build (per arch)"):
        iso_base = PredefinedProcess("just iso-base\n(debootstrap +\npackages.sh)")
        iso_rootfs = PredefinedProcess("just iso-rootfs\n(inject installer +\nmksquashfs +\nveritysetup)")
        iso_assemble = PredefinedProcess("just iso\n(xorriso hybrid +\nappend BESIMAGES +\nappend BESCONF)")
        iso_out = Storage("bes-installer.iso\n(+ .vdi)")
        iso_base >> iso_rootfs >> iso_assemble >> iso_out
        installer_bin >> Edge(label="copied into\nlive rootfs") >> iso_rootfs

    with Cluster("Pi EEPROM SD build (arch-independent)"):
        eeprom_src = Document("rpi-eeprom\nupstream image")
        eeprom_build = Bash("image/build-pi-eeprom-sd.sh\n(rpi-eeprom-config edit +\nMBR + FAT16 image)")
        eeprom_out = MultipleDocuments("loose files\n+ flashable .img")
        eeprom_src >> eeprom_build >> eeprom_out

    ci >> installer_bin
    ci >> image_build
    ci >> iso_base
    ci >> eeprom_build

    raw >> Edge(label="cloud variant\nis the source for\nthe ISO image squashfs", style="dotted") >> iso_assemble

    with Cluster("Release outputs"):
        release_disk = Storage("disk images\n(metal/cloud/pi)")
        release_iso = Storage("ISO + VDI")
        release_eeprom = Storage("Pi EEPROM SD")
        release_aws = Server("AWS AMI\nregistration\n(tagged release)")

    post >> release_disk
    iso_out >> release_iso
    eeprom_out >> release_eeprom
    release_disk >> Edge(label="cloud only,\non tag") >> release_aws
