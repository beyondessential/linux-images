"""TUI screen graph for the installer.

Sub-screens (network check, Tailscale auth-key, SSH keys, GitHub import,
debug shell) hang directly off their parent screens along the main flow,
using dotted edges so the linear path stays visually dominant.

Render with: uv run python installer-tui.py
"""

from diagrams import Diagram, Edge
from diagrams.programming.flowchart import Display, StartEnd

graph_attr = {
    "fontsize": "18",
    "labelloc": "t",
    "rankdir": "LR",
    "splines": "spline",
    "nodesep": "0.4",
    "ranksep": "0.7",
}

with Diagram(
    "Installer TUI: screen graph",
    show=False,
    filename="installer-tui",
    outformat="png",
    graph_attr=graph_attr,
):
    enter = StartEnd("Enter TUI")
    welcome = Display("Welcome\n(verity check\nin background)")
    debug_shell = Display("Debug shell\n(Ctrl+Alt+d,\navailable everywhere)")
    net_cfg = Display("Network Config\n(ISO + target,\naccordion)")
    net_check = Display("Network Check\n(Alt+c)")
    disk_sel = Display("Disk Selection")
    disk_enc = Display("Disk Encryption\n(TPM /\nkeyfile / none)")
    host = Display("Hostname\nselect\n(static / DHCP)")
    host_input = Display("Hostname Input")
    login = Display("Login\n(password +\nAlt+t/s/g)")
    ts_sub = Display("Tailscale\nauth key\n(Alt+t)")
    ssh_sub = Display("SSH keys\n(Alt+s)")
    gh_sub = Display("GitHub import\n(Alt+g,\nif github.com\nreachable)")
    tz = Display("Timezone")
    net_results = Display("Network Results")
    confirm = Display("Confirmation\n(generates\nrecovery passphrase)")
    installing = Display("Installing\n(progress bar)")
    error = Display("Error screen\n(any fatal step;\npress key\nto reboot)")
    done = Display("Done\n(passphrase\nshown again)")

    enter >> welcome >> net_cfg >> disk_sel >> disk_enc >> host
    host >> Edge(label="static") >> host_input >> login
    host >> Edge(label="DHCP") >> login
    login >> tz >> net_results >> confirm >> installing >> done

    welcome >> Edge(label="Ctrl+Alt+d", style="dotted", dir="both") >> debug_shell

    net_cfg >> Edge(label="Alt+c", style="dotted", dir="both") >> net_check

    login >> Edge(label="Alt+t", style="dotted", dir="both") >> ts_sub
    login >> Edge(label="Alt+s", style="dotted", dir="both") >> ssh_sub
    ssh_sub >> Edge(label="Alt+g", style="dotted", dir="both") >> gh_sub

    installing >> Edge(label="fatal", color="red", style="dotted") >> error
