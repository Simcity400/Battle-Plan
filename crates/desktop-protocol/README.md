# desktop-protocol

The small shared contract between the Tauri desktop host and native viewer. Keep it focused on control messages and the recording revision marker; it is not a replay-data or rendered-image transport.

## What crosses the boundary

The host sends geometry, visibility, open-recording and seek commands over a line-oriented pipe. Geometry and other replaceable updates may be coalesced. Preserve command ordering where it changes behavior, and keep both endpoints compatible in the same change.

The revision marker uses file length and modification time. It detects the recording revision expected by a query result; it is not a cryptographic content identity. A map seek based on an outdated analytics result must be refused after a file change.

Do not send map images or replay tables through this protocol. Analytics reads the recording through its own worker; the viewer renders through its own native pipeline.

## Changing the contract

Start in [src](src/), then update [desktop](../desktop/README.md) and [viewer](../viewer/README.md) together. Keep the types small, validate malformed input through existing tests, and follow the [shared checks](../../AGENTS.md). Window ownership, DPI, shutdown and child-process cleanup remain endpoint responsibilities.
