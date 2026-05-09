"""Top-level catalogue of release artifacts and where each one ends up.

Each tagged release on GitHub publishes a fixed set of artifacts.
This diagram groups them by family and shows the runtime targets.

Render with: uv run python artifacts.py
"""

from diagrams import Cluster, Diagram, Edge
from diagrams.aws.compute import AMI
from diagrams.generic.compute import Rack
from diagrams.generic.device import Mobile, Tablet
from diagrams.generic.os import Raspbian, Ubuntu
from diagrams.generic.storage import Storage
from diagrams.generic.virtualization import Qemu, Virtualbox, Vmware
from diagrams.onprem.vcs import Github
from diagrams.programming.flowchart import MultipleDocuments

graph_attr = {
    "fontsize": "18",
    "labelloc": "t",
    "rankdir": "LR",
    "splines": "spline",
    "nodesep": "0.4",
    "ranksep": "1.2",
}

with Diagram(
    "BES Linux Images: artifact catalogue",
    show=False,
    filename="artifacts",
    outformat="png",
    graph_attr=graph_attr,
):
    base = Ubuntu("Ubuntu base\n(noble)")
    release = Github("GitHub release\n(per tag)")
    base >> release

    with Cluster("Cloud variant"):
        cloud_imgs = Storage("img.zst /\nvmdk / qcow2")

    with Cluster("Metal variant"):
        metal_imgs = Storage("img.zst /\nvmdk / qcow2")

    with Cluster("Pi variant (arm64)"):
        pi_img = Storage("img.zst")

    with Cluster("Live ISO"):
        iso = Storage(".iso (hybrid)\n+ .vdi for VBox")

    with Cluster("Pi EEPROM SD"):
        eeprom_files = MultipleDocuments("loose files")
        eeprom_img = Storage("flashable .img")

    release >> [cloud_imgs, metal_imgs, pi_img, iso, eeprom_files, eeprom_img]

    with Cluster("Runtime targets"):
        aws = AMI("AWS AMI\n(auto-registered)")
        kvm = Qemu("KVM /\nlibvirt /\nProxmox")
        vmware = Vmware("VMware /\nvSphere")
        vbox = Virtualbox("VirtualBox\n(testing)")
        bare = Rack("Bare metal")
        usb = Tablet("USB stick")
        pi5 = Raspbian("Raspberry Pi 5")
        pi_boot = Mobile("Pi 5 EEPROM\n(reflashed)")

    cloud_imgs >> Edge(label="aws import-snapshot") >> aws
    cloud_imgs >> kvm
    cloud_imgs >> vmware
    metal_imgs >> Edge(label="dd / install") >> bare
    metal_imgs >> kvm
    metal_imgs >> vmware
    pi_img >> Edge(label="dd to SD/USB/NVMe") >> pi5
    iso >> Edge(label="dd") >> usb
    iso >> vbox
    usb >> Edge(label="installer writes\nmetal/cloud onto", style="dotted") >> bare
    eeprom_img >> Edge(label="dd to SD,\nboot once") >> pi_boot
    eeprom_files >> Edge(label="copy onto FAT SD,\nboot once", style="dashed") >> pi_boot
