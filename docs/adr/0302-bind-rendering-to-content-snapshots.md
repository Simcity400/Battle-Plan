# 0302. Bind dashboard work to one content snapshot

Artwork selection is a presentation identity shared by a recording's widgets, catalogue, artwork cache and stored replies. It includes the campaign's selected visual mods by unique ID and a revision of what the installation can currently mount. Selection order has no effect on identity because the engine determines mount order. Installation changes can create a new identity with the same list of selected mods.

Each request keeps the immutable source binding it was admitted under. The binding combines the folders, cache, whether unverified content is allowed, and the presentation. Reconfiguration swaps these together so an admitted request cannot mix old folders with new mods. Each binding owns one lazily computed mount. The mount runs outside the source lock. A waiter watches its own cancellation, and a failed or interrupted computation leaves the cell available for the next caller. Retiring a binding cancels read-ahead work against it. Shutting down prevents in-flight configuration work from installing results. Reconfiguration retries a computation only when its configuration generation has changed.

Visual mods overlay artwork. Recorded facts still come from the recording's content stack. Missing visual mod art falls back by name to recorded art. Content replies carry the standing refusals for the selected mods they cannot show, with separate diagnostics when an artwork read falls back. A request resolves its needed mount on entry, before acquiring locks. Preparation uses the same folders under an empty presentation identity and reads the recording's own artwork.

[Presentation and binding](../../crates/dashboard-core/src/presentation.rs)
