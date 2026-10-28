# libherdr_term

`libherdr_term` attaches flankr's daemon to the local herdr as one of
herdr's own shell clients, over herdr's stable endpoint contract
(generation 1) on `herdr-client.sock`. It hands the daemon the session
pane's cells, cursor, size and mouse mode, and sends the pane keys, text,
paste, mouse and wheel events. herdr is not changed: its protocol types are
compiled from its own source in the `vendor/herdr` submodule.

The plan is
[libherdr_term, attaching to herdr as a client from herdr's own protocol types](../../docs/design/20261027_08_HERDR_TERMINAL_CRATE_PLAN.md).

## Building

`build.rs` reads herdr's `src/protocol/wire.rs`, `src/protocol/endpoint.rs`,
`src/input/model.rs`, `src/api/schema/common.rs` and `src/config/model.rs`
from `../../vendor/herdr` (or from `HERDR_SRC`), keeps the protocol's type
definitions, constants and framing functions, drops herdr's `impl` blocks
and test code, and writes `$OUT_DIR/herdr_wire.rs`, which the crate
includes as `libherdr_term::herdr`. Without the submodule the build stops
with the fix:

```text
git submodule update --init vendor/herdr
```

To review the extracted types when the pin moves:

```text
cargo run -p libherdr_term --example dump_herdr_types
```

`libherdr_term::HERDR_PROTOCOL_VERSION` is herdr's `PROTOCOL_VERSION` at
the pin (22 at `2713feb`), `ENDPOINT_PROTOCOL_GENERATION` the endpoint
generation spoken (1).

## Use

```rust,ignore
let socket = libherdr_term::client_socket_for(&api_socket);
let (attachment, events) = Attachment::connect(&AttachOptions::new(socket, 80, 24))?;
attachment.focus_pane("w1:p1")?;
for event in events {
    if let Event::Surface(view) = event {
        if let Some(pane) = view.pane("w1:p1") {
            let grid: libflankr_term::Grid = pane.into();
        }
    }
}
```

`cargo run -p libherdr_term --example attach -- <pane_id> 80 24` prints a
pane's rows and cursor while herdr lays its tab out at 80 by 24; herdr
gives the tab back when it exits.

Tests run against an in-process fake herdr only; nothing in them touches a
real herdr socket.

## Licence of the generated code

`libherdr_term::herdr` is generated from herdr
(<https://github.com/herdrdev/herdr>), Copyright the herdr authors,
licensed under the Apache License, Version 2.0. The licence text is
`vendor/herdr/LICENSE` in this repository's submodule. The generated code
is herdr's own definitions, unchanged except that `impl` blocks, test
items, `bincode::Decode` and `schemars::JsonSchema` derives are removed and
herdr's crate paths are rewritten to the flattened module. The rest of this
crate is flankr's.
