# collector-economy

Owned economy and fuel-demand capture, raw staging/decoding and replay projection. The host schedules acquisition and supplies snapshots and native sources. Each capture owns its rows, refusal outcomes, arenas and growth.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-economy` and `cargo clippy -p collector-economy --all-targets -- -D warnings` for its synthetic checks.

The existing resource census supplies new market country holders; its existing
owning contract census supplies new contracts. Already-held entities add no
native market read. Event storage demand uses contract pool counts already in
the raw contract bulk, alongside live schedule counts prepared during sizing.

The hourly resource census also completes a new market epoch's contract seed when
the daily owning-contract census has not run. It visits only that missing owned
contract list, once per recovery epoch; it neither advances the other daily
products nor rereads carried contracts. Cold attach and spool-gap recovery thus
need no daily-slot wait. Failed reads remain named market table refusals.
