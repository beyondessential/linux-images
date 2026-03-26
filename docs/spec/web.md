# Web UI

## Server

r[web.server.embedded]
The installer must embed an HTTP and WebSocket server that starts
automatically alongside the TUI, unless disabled by
`r[web.server.disabled]`. The server must listen on a fixed TCP port and
accept connections from any interface. The server must not block the
installer's main event loop under any circumstances.

> r[web.server.disabled]
> The web server must not start when any of the following conditions are
> true:
> 
> - The `web` configuration field is set to `false`
>   (see `r[installer.config.web]`).
> - The installer is running in automatic mode (`auto = true`).
> - The installer is running in dry-run mode (`--dry-run`).
> 
> When the web server is disabled, the installer must behave identically
> to how it behaved before the web UI feature was added: only the TUI (or
> automatic/dry-run output) is active. No TCP port is bound and no
> background threads are spawned for the web server.

r[web.server.static-assets]
The server must serve the browser client application (HTML, JavaScript,
and WebAssembly assets) over plain HTTP. These assets must be available
on the live ISO filesystem. Static assets must be served without
authentication so that the browser can load the client application before
the user has entered the password.

r[web.server.websocket]
The server must accept WebSocket upgrade requests. Each connected browser
communicates with the installer exclusively over a single WebSocket
connection. The server must handle multiple simultaneous WebSocket
connections.

r[web.server.lifecycle]
The server must start before the TUI enters its event loop and continue
running until the installer process exits. Errors in the server (failed
binds, broken client connections) must be logged but must not interrupt
the installation flow or crash the installer.

## Authentication

r[web.auth.password-source]
The web UI must be protected by a password. The password may be set via
the `web-password` field in the configuration file (`bes-install.toml`).
If the field is absent, the installer must generate a random
human-readable password at startup.

r[web.auth.generated-password]
When the password is generated automatically, it must be generated from
a cryptographically secure random source and be practical to communicate
verbally (e.g. a short sequence of dictionary words or alphanumeric
characters).

r[web.auth.tui-visibility]
The TUI must provide a way for the local user to view the current web UI
password on demand. This is necessary so that the person at the physical
terminal can communicate the password to a remote user. The password must
not be permanently visible -- it must be revealed by an explicit action.

r[web.auth.browser-visibility]
Once authenticated, the browser client must also provide a way to view
the current web UI password on demand, using the same reveal mechanism as
the TUI.

r[web.auth.websocket-gate]
The server must not send frame data or accept input events over a
WebSocket connection until the client has successfully authenticated.
The client must send the password as its first message after the
WebSocket handshake. If the password is incorrect, the server must
close the connection with an appropriate error.

r[web.auth.config-field]
The `web-password` field in the configuration file is a string. It is
optional. It must follow the same top-level, no-tables convention as
other configuration fields.

## Frame Streaming

r[web.frames.server-rendered]
All UI rendering must happen on the server (the installer process). The
server renders the current state into a ratatui buffer and serializes the
cell data for transmission to connected browsers. The browser client does
not perform any UI layout or state logic -- it only displays the cell
grid it receives.

r[web.frames.full]
The server must send full frame snapshots to each connected client. A
frame snapshot contains the terminal dimensions (columns and rows) and
the complete cell grid in row-major order. Each cell must include its
display symbol, foreground colour, background colour, and text modifier
flags (bold, italic, underline, reversed).

r[web.frames.rate]
The server must send a frame to each client whenever the UI state changes
or at a bounded maximum rate, whichever is less frequent. Unchanged
frames must not be sent repeatedly.

r[web.frames.compression]
Frame data must be compressed before transmission to reduce bandwidth.
The compression format must be supported by both the server (native) and
the browser client (WebAssembly).

## Input

r[web.input.key-events]
The browser client must capture keyboard events and send them to the
server over the WebSocket connection. The server must translate received
key events into the same input representation used by the TUI and feed
them into the shared state machine.

r[web.input.abstraction]
The installer must define an abstract input event type that is independent
of any terminal library. Both the TUI driver (translating from terminal
key events) and the web driver (translating from browser key events) must
produce instances of this shared type. The state machine and input handler
must operate on the abstract type, not on terminal-library-specific types.

## Session Management

r[web.session.multi-viewer]
Multiple browsers may connect simultaneously. All connected clients must
receive the same frame stream reflecting the current installer state.

r[web.session.single-controller]
At any given time, at most one party has input control. Input events from
all other parties must be ignored. The TUI terminal counts as a party for
the purposes of control.

r[web.session.take-control]
Any party (browser client or TUI) may take input control at any time
without requiring permission from the current controller. Taking control
is instantaneous: the previous controller becomes a watcher and the new
controller's input events take effect immediately.

r[web.session.initial-control]
When the installer starts, the TUI terminal must have input control by
default.

r[web.session.disconnect]
When a WebSocket client disconnects, it must be removed from the session.
If the disconnecting client had input control, control must revert to the
TUI terminal.

## Status Indicators

r[web.status.watcher-count]
The server must include metadata alongside each frame indicating how many
parties (browser clients plus the TUI) are currently connected.

r[web.status.control-indicator]
Each party must be informed whether it currently has input control or is a
watcher. The frame metadata must include sufficient information for the
client to display this status.

r[web.status.tui-indicator]
When the TUI terminal does not have input control, the TUI must display a
visual indicator that a remote user is in control, along with a hint for
how to take control back.

r[web.status.browser-indicator]
When a browser client is a watcher, it must display a visual indicator
that it is watching, along with a hint for how to take control.

## Browser Client

r[web.client.rendering]
The browser client must render the received cell grid using a
GPU-accelerated terminal renderer (WebGL2). The visual appearance must
closely match the TUI as rendered on a physical terminal.

r[web.client.font-atlas]
The browser client must ship a monospace font atlas suitable for terminal
rendering. The atlas must cover at least printable ASCII, which is
sufficient given `r[installer.tui.ascii-rendering]`.

r[web.client.connection]
The browser client must open a WebSocket connection to the server on page
load. If the connection is lost, the client must display a disconnection
notice and attempt to reconnect.

r[web.client.separate-build]
The browser client must be a separate build artifact targeting
WebAssembly. It must not share a compilation unit with the native
installer binary.

## State Architecture

r[web.state.pure-model]
The installer's UI state (`AppState`) must be a pure data structure with
no I/O handles, channel endpoints, or thread-specific resources. All
asynchronous I/O receivers (network check results, background fetch
results) must be held outside the state struct by the driver layer. This
separation is required so that the state can be cloned for frame
rendering and shared across drivers.

r[web.state.shared-render]
The same render function must be used for both the TUI terminal and the
web frame stream. There must not be a separate rendering path for web
clients.

r[web.state.single-state]
There must be exactly one `AppState` instance driving all connected
parties. The TUI and all browser clients see the same state at all times.
