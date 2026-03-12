# Web UI Implementation Plan

This document describes the implementation plan for embedding an HTTP server
in the installer to provide a web-based interface to the installation process.
The corresponding spec is in `docs/spec/web.md`.

## Architecture Overview

The installer already has a clean Elm-like architecture:

- `AppState` (model) -- pure data struct with state transition methods
- `handle_key` (update) -- maps key events to state mutations + actions
- `render` (view) -- pure function from `&AppState` to ratatui widgets

The web UI feature introduces a second rendering target (browser) alongside
the existing terminal, driven by the same state. The server renders frames
into a ratatui buffer on behalf of all connected browsers and streams the
cell data over WebSocket.

```
                        ┌──────────────────┐
                        │    AppState       │
                        │  (single source   │
                        │   of truth)       │
                        └────────┬─────────┘
                                 │
                    ┌────────────┴────────────┐
                    │                         │
              ┌─────┴──────┐          ┌───────┴───────┐
              │ TUI Driver │          │  Web Driver   │
              │ crossterm  │          │ TCP listener  │
              │ terminal   │          │ WebSocket per │
              │            │          │ client        │
              └────────────┘          └───────────────┘
```

## Dependencies

Server side (installer binary):
- `tungstenite` -- synchronous WebSocket library (no async runtime needed)
- `flate2` -- deflate compression for frame data (pure-Rust `miniz_oxide`
  backend, no C dependency)
- `diceware_wordlists` -- already in the dependency tree, used for
  generating the web password (same as recovery passphrase generation)

Browser side (separate WASM crate):
- `beamterm-renderer` -- GPU-accelerated terminal rendering via WebGL2
- `flate2` -- compiled to WASM for decompressing frames (the `miniz_oxide`
  backend is pure Rust and compiles cleanly to `wasm32-unknown-unknown`
  with no special configuration)
- Built with `trunk` or `wasm-pack`

Compression rationale: the `zstd` crate was previously in the dependency
tree but was removed when the installer switched to splice-based image
writing with squashfs+verity (kernel handles decompression). Rather than
reintroduce a C dependency, we use `flate2` with the `miniz_oxide`
backend. It is pure Rust on both sides (native and WASM), well-maintained,
and deflate compresses repetitive terminal data effectively. Terminal
frames are small enough that compression ratio differences between deflate
and zstd are negligible.

No async runtime (tokio, etc.) is introduced. The existing codebase is
entirely synchronous with `std::thread` + `mpsc`, and the web driver fits
into this model: the TCP listener runs in its own thread, each WebSocket
client gets a thread, and communication with the main event loop uses
`mpsc` channels.

## Refactoring Phases

### Phase 1: Purify AppState

**Goal:** Make `AppState` free of I/O handles so it can be cloned/shared.

Currently `AppState` contains four `Option<mpsc::Receiver<...>>` fields:
- `verity_rx` (integrity check progress/completion)
- `net_check_rx` (network connectivity check results)
- `netcheck_rx` (tailscale netcheck result)
- `ssh_github_rx` (GitHub SSH key fetch result)

It also contains one `Option<Instant>` field:
- `net_apply_debounce` (ISO netplan apply timer)

The receivers prevent `AppState` from being `Clone` or `Send`. The
`Instant` is `Clone` and `Send` so it is not a blocker, but logically it
is a timer side-effect that belongs with the driver, not the model.

**Changes:**
1. Create a `SideEffects` struct that holds the four receivers and the
   debounce `Instant`.
2. Move the `start_*` methods so they return receivers instead of storing
   them in `AppState`. The caller (event loop / driver) stashes them in
   `SideEffects`.
3. Move the `poll_*` methods and `poll_iso_apply_debounce` from `AppState`
   to `SideEffects`, taking `&mut AppState` as a parameter.
4. After this, `AppState` becomes `Clone` (all remaining fields are
   `String`, `Vec<_>`, `Option<_>`, enums, primitive types, etc.).

The `Screen::Error(String)` variant is already `Clone`. The remaining
types (`BlockDevice`, `NetInterface`, `StaticNetConfig`, etc.) are all
composed of `Clone` primitives.

All existing tests should pass with minimal changes -- the test helpers
and `run_tui_scripted` just need to thread `SideEffects` alongside
`AppState`.

Commit this phase independently.

### Phase 2: Abstract Input Events

**Goal:** Decouple input handling from `crossterm::event::KeyEvent`.

The current `handle_key` function in `ui/run.rs` takes a
`crossterm::event::KeyEvent` and matches on `KeyCode` and `KeyModifiers`.
It filters `KeyEventKind::Press` at the top. The match arms cover: chars,
Enter, Esc, Tab, BackTab (and Alt+Tab as a Linux TTY workaround for
Shift+Tab), Backspace, Up, Down, and modifier combinations (Alt+char,
Ctrl+Alt+char).

**Changes:**
1. Define a crate-level `InputEvent` enum that captures only what
   `handle_key` actually matches on: key code (char, enter, esc, tab,
   backtab, backspace, up, down) and modifiers (alt, ctrl).
2. Implement `From<crossterm::event::KeyEvent> for InputEvent`, including
   the Alt+Tab-to-BackTab normalization currently done inline.
3. Change `handle_key` to take `InputEvent` instead of `KeyEvent`. The
   `KeyEventKind::Press` filter moves into the `From` impl (non-Press
   events map to a `None` / ignored variant).
4. On the web side, the browser sends key events as JSON over WebSocket,
   which deserializes directly to `InputEvent`.

This is a mostly mechanical change -- the match arms in `handle_key` stay
the same, just the input type changes.

Commit this phase independently.

### Phase 3: Render to Virtual Buffer

**Goal:** Allow rendering into a `ratatui::buffer::Buffer` without a live
terminal.

**Changes:**
1. Add a `render_to_buffer(state, width, height) -> Buffer` function that
   creates a buffer-backed frame, calls the existing `render()`, and
   returns the buffer.
2. The existing `render(frame, state)` function is unchanged -- it already
   takes `&mut Frame` and `&AppState` with no terminal coupling.
3. The TUI driver continues using `terminal.draw(|f| render(f, state))`.
4. The web driver calls `render_to_buffer()` and serializes the result.

This is a small addition, not a refactor. The existing render path is
untouched.

Commit this phase independently.

### Phase 4: Wire Protocol and Server

**Goal:** Embed an HTTP + WebSocket server in the installer.

**Server enable/disable:**
The server is only started when all of the following are true:
- The installer is running in interactive or prefilled mode (not `auto`).
- The `--dry-run` flag is not set.
- The `web` config field is not explicitly set to `false`.

The check happens in `RunContext::run_interactive()` before the event
loop starts. When disabled, no `TcpListener` is bound, no threads are
spawned, and the installer behaves exactly as it did before the web UI
feature existed. The `web-password` config field is also ignored when
the server is disabled.

**Server architecture:**
1. A `TcpListener` bound to `0.0.0.0:8080` runs in a dedicated thread.
2. On each connection, read the first bytes to determine request type:
   - HTTP request for static files (HTML, WASM, JS) -- serve from
     embedded or filesystem assets, then close.
   - WebSocket upgrade -- perform the tungstenite handshake, then spawn
     a dedicated thread for this client.
3. Each WebSocket client thread:
   - Receives rendered frames from the main loop via a broadcast mechanism
     (e.g. a shared buffer behind `Arc<Mutex<>>` with a condition variable,
     or per-client `mpsc` senders).
   - Sends deflate-compressed binary frame messages to the browser.
   - Reads key events from the browser and forwards them to the main loop
     via an `mpsc::Sender<InputEvent>`.

**Wire format (server to browser):**
- Binary WebSocket message = deflate-compressed(frame_header + cell_data[])
- Frame header: cols (u16), rows (u16), watcher count (u16), control
  status (u8), plus reserved bytes for future use.
- Cell data: row-major array, one entry per cell. Each entry contains the
  symbol (UTF-8, length-prefixed or null-terminated), foreground RGB (3
  bytes), background RGB (3 bytes), and modifier bits (1 byte for bold,
  italic, underline, reversed).
- Compression: raw deflate via `flate2::write::DeflateEncoder` on the
  server, `flate2::write::DeflateDecoder` (or `DeflateDecoder` reader)
  on the WASM client.

**Wire format (browser to server):**
- JSON text WebSocket messages for simplicity (input events are tiny and
  infrequent).
- Authentication: `{"type":"auth","password":"word-word-word"}`
- Key events: `{"type":"key","code":"Enter","alt":false,"ctrl":false}`
- Control requests: `{"type":"take_control"}`

**Authentication handshake:**
- After the WebSocket connection is established, the client must send an
  `auth` message as its first message.
- The server validates the password against the known web password. If
  correct, the client is admitted and begins receiving frames. If
  incorrect, the server closes the connection with a close frame
  containing an error reason.
- No frames or input events are processed before successful auth.

**Main loop integration:**
- The main event loop gains an `mpsc::Receiver<InputEvent>` from the web
  driver, checked each tick alongside the crossterm event poll.
- After each render, the buffer is serialized and broadcast to all
  connected WebSocket clients.
- The web server thread and client threads never block the main loop --
  all communication is via non-blocking `try_recv()`.

**Static assets:**
- The HTML page, WASM binary, and JS glue are served from a known path
  on the ISO filesystem (e.g. `/run/live/medium/web/`) or embedded in
  the installer binary.

### Phase 5: Browser Client (Separate Crate)

**Goal:** Build a WASM application that displays terminal frames and
captures input.

**Crate setup:**
- New crate at `installer/web-client/` targeting `wasm32-unknown-unknown`.
- Added to the workspace `members` list in the root `Cargo.toml`.
- Depends on `beamterm-renderer` (WebGL2 backend) for GPU-accelerated
  terminal rendering. Does NOT depend on ratatui -- all rendering happens
  server-side.
- Depends on `flate2` (pure-Rust `miniz_oxide` backend) for decompressing
  frames. This compiles cleanly to WASM with no feature flags or wasm-
  specific workarounds.
- Built with `trunk`, producing `index.html` + `.wasm` + `.js`.

**Client logic:**
1. Open a WebSocket to the server (same origin, `/ws` path).
2. Create a beamterm `Terminal` with the WebGL2 backend + static font
   atlas (the default Hack font atlas embedded in beamterm).
3. On each binary message: inflate (raw deflate), decode cell data,
   convert to beamterm `CellData` array, call `terminal.update_cells()`
   and `terminal.render_frame()`.
4. On keyboard events: serialize as JSON, send over WebSocket.
5. Render a status overlay showing connection state, watcher count, and
   control status.

The conversion from our wire cell format to beamterm `CellData` follows
the same logic that ratzilla uses in its WebGL2 backend (`cell_data()` /
`into_glyph_bits()`).

### Phase 6: Authentication

**Goal:** Protect the web UI with a password.

**Password source:**
- If `web-password` is set in `bes-install.toml`, use that value.
- Otherwise, generate a random password at startup using the same
  diceware approach as `generate_recovery_passphrase()` but shorter
  (3-4 words instead of 6) since this password only protects a
  transient session, not encrypted data.

**Password storage:**
- The password is stored in the `RunContext` or a shared `WebConfig`
  struct (behind `Arc<str>` or similar) so the web server threads can
  read it.

**WebSocket authentication flow:**
1. Client connects, WebSocket handshake completes.
2. Server waits for the first text message. If it is not a valid `auth`
   message, or if the password is wrong, the server sends a close frame
   and drops the connection.
3. On successful auth, the client is registered with the
   `SessionManager` and begins receiving frames.

**TUI visibility:**
- The web password is part of `AppState` (it's just a `String`, no I/O
  coupling). The render function can display it.
- A keybind (e.g. `w` on the welcome screen, or a global keybind shown
  in the footer) toggles password visibility. When revealed, the
  password and the server's listen address are shown in a small overlay
  or dedicated line, so the user at the terminal can read it aloud or
  copy it.
- The password is hidden again on the next keypress or after a timeout.

**Browser visibility:**
- Once authenticated, the browser client has the password (it just
  entered it). The client stores it locally and provides the same
  reveal-on-demand mechanism, so the browser user can share it with
  another person.

Commit this phase independently -- it can be developed alongside or
immediately after Phase 4.

### Phase 7: Session and Control Management

**Goal:** Support multiple concurrent viewers with single-controller
semantics.

**Design:**
- A `SessionManager` struct (behind `Arc<Mutex<>>`) tracks all connected
  WebSocket clients and which one has input control.
- On connect, a client is a "watcher" -- it receives frames but its key
  events are ignored.
- Sending a `take_control` message makes the sender the controller. The
  previous controller becomes a watcher. No permission check -- control
  is freely takeable.
- The local TUI terminal always has implicit control. When a web client
  takes control, the TUI still renders but its key events are ignored
  (with an on-screen indicator). The TUI user can take control back by
  pressing a designated key.
- Frame metadata includes watcher count and control status so each client
  can display appropriate indicators.

## Build Changes

### Release Profile

No changes needed. The old size-optimized `[profile.release]` that was in
`installer/tui/Cargo.toml` has already been removed on main. The default
Cargo release profile (`opt-level = 3`) is used. If we want LTO or other
tuning, it should go in the workspace root `Cargo.toml`.

### Web Client Build

The web client is a separate crate added to the workspace:

```
# Root Cargo.toml
[workspace]
members = ["installer/tui", "installer/web-client"]
resolver = "3"
```

The justfile gains a recipe to build it with `trunk`:

```
installer-web-build:
    cd installer/web-client && trunk build --release
```

The resulting `dist/` directory contents are either:
- Copied to the ISO filesystem at build time, or
- Embedded into the installer binary via `include_bytes!` (deferred
  decision -- filesystem is simpler to start with).

## Bandwidth Estimate

A 200x50 terminal = 10,000 cells. At ~10 bytes/cell raw = 100KB per
frame. Terminal data is highly repetitive (spaces, repeated border
characters, same colors), so deflate compression should achieve 10:1 or
better, yielding ~10KB per frame. At 20fps = 200KB/s -- trivial even on
constrained networks.

Full frames (no diffing) are used initially for simplicity. Diff-based
updates can be added later if needed but are unlikely to be necessary
given the compression ratio.

## Testing Strategy

- **Phase 1-2:** All existing unit tests continue to pass. The refactors
  are validated by the existing test suite (scripted TUI tests via
  `run_tui_scripted`, state machine tests, render tests).
- **Phase 3:** Add a test that renders each screen to a buffer and
  verifies dimensions / non-empty content. Similar to the existing
  `*_ascii_only` render tests.
- **Phase 4:** Integration test that starts the server, connects a
  WebSocket client, receives a frame, sends a key event, and verifies
  the state changed. Can run headless (no terminal needed).
- **Phase 5:** The WASM client is tested manually in a browser during
  development. Automated testing deferred (browser automation is heavy).
- **Phase 6:** Unit tests for password generation (length, format).
  Integration test that verifies: unauthenticated WebSocket is closed,
  wrong password is rejected, correct password admits the client.
  Test that config-provided password is used when present.
- **Phase 7:** Unit tests for `SessionManager` (take control, watcher
  count, disconnect cleanup).

## Order of Work

Phases 1-3 are independent refactors that improve the codebase regardless
of the web UI feature. They can be merged incrementally.

Phase 4 is the core feature. Phase 5 can be developed in parallel once the
wire protocol is defined. Phase 6 (authentication) should be implemented
alongside or immediately after Phase 4 -- the server should never be
reachable without authentication, even during development. Phase 7 is
layered on top.