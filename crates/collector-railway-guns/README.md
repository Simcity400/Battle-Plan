# collector-railway-guns

Railway gun capture: the hourly RailwayGunCounterInputs with their route cells, stored names, tooltip scratch and ETA operands, the hourly OwnerTransportSpeeds (for each positive logical owner of a walked gun, the transport speed of every strategic region its guns stand in or route through, one batched query per owner sharing each region's identity and weather and each navy's regional operands across owners), and the name sizing hints the host's sizing polls raise. Supply and combat reference inputs come from collector-division-parts. The host owns dispatch, loans and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

Run `cargo test -p collector-railway-guns` and `cargo clippy -p collector-railway-guns --all-targets -- -D warnings` for its synthetic checks.

OrdinaryMap uses `acquire_with_hour`: the parked worker retains native-demand
sorts, exact native reads, shared-memo admission and first refusals. The FIFO
decoder evaluates owned transport speed operands, counts row diagnostics and
releases copied navy arrays through `decode_owned`, which accepts no Memory.
Projection refuses to use unsettled transport operands. Five live-sized demand
vectors retain their scratch capacity across hours. The gun row prefix stays
available to the same-pause supply consumer; no new cross-frame loan is needed.
