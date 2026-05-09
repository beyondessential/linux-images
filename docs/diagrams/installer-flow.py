"""Installer high-level flow: modes -> TUI -> write -> finalise -> reboot.

Each macro phase is a single node here. The TUI screen sequence and the
post-write phases are detailed separately in `installer-tui.py` and
`installer-write.py` respectively.

Render with: uv run python installer-flow.py
"""

from diagrams import Cluster, Diagram, Edge
from diagrams.generic.storage import Storage
from diagrams.onprem.security import Vault
from diagrams.programming.flowchart import (
    Action,
    Decision,
    Document,
    InputOutput,
    PredefinedProcess,
    StartEnd,
)

graph_attr = {
    "fontsize": "18",
    "labelloc": "t",
    "rankdir": "LR",
    "splines": "spline",
    "nodesep": "0.5",
    "ranksep": "1.0",
}

with Diagram(
    "Installer: high-level flow",
    show=False,
    filename="installer-flow",
    outformat="png",
    graph_attr=graph_attr,
):
    start = StartEnd("bes-installer\nautostarted in\nlive ISO")

    with Cluster("Startup"):
        cdrom = Action("CD-ROM partscan\n(loop dev fallback)")
        besconf_mount = Action("Mount BESCONF\nro -> rw remount probe")
        config_load = Decision("config + auto?")

    start >> cdrom >> besconf_mount >> config_load

    with Cluster("Modes"):
        mode_interactive = InputOutput("Interactive\nTUI")
        mode_prefilled = InputOutput("Prefilled\nTUI")
        mode_auto = InputOutput("Automatic\n(no TUI)")

    config_load >> Edge(label="no config") >> mode_interactive
    config_load >> Edge(label="auto=false") >> mode_prefilled
    config_load >> Edge(label="auto=true") >> mode_auto

    tui = PredefinedProcess("TUI screens\n(see installer-tui.png)")
    [mode_interactive, mode_prefilled] >> tui

    write_phase = PredefinedProcess("Write + UUID + LUKS\n(see installer-write.png)")
    encryption = PredefinedProcess("Encryption setup\n(TPM / keyfile)")
    rebuild = PredefinedProcess("Rebuild boot config\n(dracut + grub\nin chroot)")
    finalise = PredefinedProcess("Install-time config\n(hostname, tailscale,\nSSH, password,\ntimezone, network)")
    reboot = StartEnd("Unmount, close LUKS,\nreboot (or --no-reboot)")

    tui >> write_phase
    mode_auto >> write_phase
    write_phase >> encryption >> rebuild >> finalise >> reboot

    pass_doc = Vault("recovery passphrase\n(in-memory, also\nshown to user)")
    tui >> Edge(label="generate", style="dashed") >> pass_doc
    pass_doc >> Edge(style="dashed") >> write_phase

    with Cluster("Side artifacts"):
        plan = Document("install-plan.json\n(--dry-run)")
        keys_file = Storage("recovery-keys.txt\non BESCONF")
        fail_log = Storage("installer-failed.log\non BESCONF")

    tui >> Edge(label="--dry-run", style="dotted") >> plan
    finalise >> Edge(label="--save-recovery-keys", style="dotted") >> keys_file
    reboot >> Edge(label="on fatal error", color="red", style="dotted") >> fail_log
