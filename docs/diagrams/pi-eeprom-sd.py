"""Pi 5 EEPROM SD: a single-purpose SD-card artifact that reflashes the
bootloader EEPROM with a known config, then reboots.

Render with: uv run python pi-eeprom-sd.py
"""

from diagrams import Cluster, Diagram, Edge
from diagrams.generic.os import Raspbian
from diagrams.generic.storage import Storage
from diagrams.programming.flowchart import (
    Action,
    Document,
    InternalStorage,
    MultipleDocuments,
    StartEnd,
)
from diagrams.programming.language import Bash

graph_attr = {
    "fontsize": "18",
    "labelloc": "t",
    "rankdir": "LR",
    "splines": "spline",
    "nodesep": "0.4",
    "ranksep": "1.0",
}

with Diagram(
    "Pi 5 EEPROM SD: build & runtime flow",
    show=False,
    filename="pi-eeprom-sd",
    outformat="png",
    graph_attr=graph_attr,
):
    upstream = Document(
        "rpi-eeprom\nupstream image\n(pieeprom-default.bin)"
    )
    builder = Bash(
        "image/build-pi-eeprom-sd.sh\n(rpi-eeprom-config edit\nwith fixed bootconf)"
    )

    with Cluster("Bootconf overrides"):
        bootconf = Document(
            "BOOT_UART=1\nBOOT_ORDER=0xf61\n"
            "POWER_OFF_ON_HALT=1\nPCIE_PROBE=1\n"
            "BOOT_WATCHDOG_TIMEOUT=15\nHDMI_DELAY=0\n"
            "PSU_MAX_CURRENT=5000"
        )

    with Cluster("Output form 1: loose files"):
        loose = MultipleDocuments(
            "recovery.bin\npieeprom.upd\npieeprom.sig\nSHA256SUMS"
        )

    with Cluster("Output form 2: flashable image"):
        img = Storage(
            "bes-pi-eeprom-config.img\nMBR + FAT16 (label=RECOVERY)\n<= 64 MiB, single partition\nstarting at 1 MiB"
        )
        sig_inline = InternalStorage(
            "Inside RECOVERY:\nrecovery.bin +\npieeprom.upd +\npieeprom.sig"
        )
        img >> sig_inline

    upstream >> builder
    bootconf >> builder
    builder >> loose
    builder >> img

    with Cluster("Operator workflow"):
        copy_files = Action("Copy files\nonto existing\nFAT SD")
        flash_img = Action("dd flashable\nimage onto SD")
        sd = Storage("SD card")
        copy_files >> sd
        flash_img >> sd

    loose >> Edge(style="dashed") >> copy_files
    img >> Edge(style="dashed") >> flash_img

    with Cluster("Pi 5 boot"):
        pi5 = Raspbian("Pi 5 powered on\nwith SD inserted")
        eeprom_check = Action(
            "EEPROM bootloader\nreads pieeprom.sig\n(SHA-256 + ts)"
        )
        flash = Action("EEPROM flashed\nwith new image\n(if newer)")
        reboot = Action("Pi reboots\nwith new bootconf")
        verify = StartEnd(
            "vcgencmd\nbootloader-config\nreflects overrides"
        )
        pi5 >> eeprom_check >> flash >> reboot >> verify

    sd >> Edge(label="boot once") >> pi5
