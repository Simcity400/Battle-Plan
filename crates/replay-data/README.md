# replay-data

The recorded data contract: engine-native rows, typed tables, cadence, exact
values, content identity and their bounded codecs. Frame, spool and container
implementations consume these definitions; this crate depends directly on the
native libraries that own its value types and on `engine-allocation` for checked
growth. It depends on
[replay-identity](../replay-identity/README.md) for the content identity types
its `identity` module writes and reads. [weather-period](../weather-period/README.md) supplies the shared weather-period types and calendar arithmetic, re-exported by `temperature`.

| Source | Responsibility |
|---|---|
| [lib.rs](src/lib.rs), [format.rs](src/format.rs) | Shared hour/session/branch types, writer version and format admission. |
| [rows.rs](src/rows.rs), [rows/compatibility.rs](src/rows/compatibility.rs) | Table registration, row dispatch, field masks, native representations and proven wire/meaning history. |
| [extended_rows/](src/extended_rows/), domain row modules | Recorded operands, ordering, observation boundaries, completeness and named refusals. |
| [cadence.rs](src/cadence.rs), [sliced.rs](src/sliced.rs), [cadence-audit.md](cadence-audit.md) | Read scheduling, owner slices, carry rules and supplying hours; the native writers that keep each hourly table hourly. |
| [codec.rs](src/codec.rs), [value.rs](src/value.rs), [fixed_point.rs](src/fixed_point.rs), [display.rs](src/display.rs) | Bounded decoding, exact value encoding and arithmetic, and game display steps. |
| [identity.rs](src/identity.rs) | The recorded form of the content and session identity (`IdentityCodec`); the types and their validation are in [replay-identity](../replay-identity/README.md). |
| [checkpoint/](src/checkpoint/), [rows/checkpoint.rs](src/rows/checkpoint.rs) | Prepared row and value codecs, independent of frame snapshot framing. |
| [country_flags.rs](src/country_flags.rs), [day_night.rs](src/day_night.rs), [naval_trees.rs](src/naval_trees.rs), [temperature.rs](src/temperature.rs) | Shared recorded operands and exact calculations, including the operands that pick a country's flag. |

Output workers can hold `codec::error_details_scope` or use `codec::with_error_details` to retain validation reasons and row/column context. Scopes nest and restore the calling thread's previous setting on return. Outside that scope, installed capture allocation hooks keep static I/O errors allocation-free.

Row declarations own versions, field counts, keys, representations and
availability. Writers emit the current format; readers admit unchanged contracts
through the compatibility history and refuse changed tables by name. Preserve
recorded absence separately from native empty, zero or absence. Content facts
remain resolved through the recorded content stack. Shared rules are in
[AGENTS.md](../../AGENTS.md); frames are in [replay-frame](../replay-frame/README.md)
and recording I/O is in [replay](../replay/README.md).

The `test-support` feature exposes synthetic row fixtures, historical fixture
encoding and thread-local decode observations for replay's frame/container tests.
These helpers are absent from ordinary builds. Unit tests exercise round trips,
display steps, historical prefixes, delta ownership, malformed input and wide
country lists. [replay-frame](../replay-frame/README.md) owns frame and baseline tests;
[replay](../replay/README.md) owns recovery and complete recording tests. Run
`cargo nextest run -p replay-data -p replay-frame -p replay` for all three.

Terminal failure witnesses live in [witness.rs](src/witness.rs), so capture
helpers can report exact failures without depending on compressed recording I/O.
The row registry owns `rows::TABLE_COUNT`; table accounting needs no frame engine.

`AgencyOperationBuffers` holds one observed shared-client snapshot: completed, running, ready-preparation and unready-preparation references in native order, with each reference's resolved target state. Empty lists are recorded; unavailable acquisition remains a table refusal. UI membership is runtime state, independent of saved operation-roster membership.

ResourceOrigins and ResourceRights carry native list ordinals. ConvoyClients carries independent optional owner/counterparty ordinals for the directional lists; imports and exports use ResourceExchanges ordinals. Ordinary and prepared codecs retain these fields with the rest of each existing row.

TradeRoutes source_state and destination_state retain market-referenced routes' native state endpoints separately from naval ports. Both optional columns belong to the complete format-103 introduction and use ordinary delta and prepared codecs.
