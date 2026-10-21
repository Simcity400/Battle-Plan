# collector-ship-models

Ship model capture: each ship's ShipModels operands (fixed fields, pool entries, sprites, experience and medal modifiers), the held read that serves a ship until a hooked writer touches it, and the ship model writer observer. The host owns dispatch, loans and table assembly.

Read the [shared rules](../../AGENTS.md) and the [collector guide](../collector/README.md).

ShipModelDetails freezes read demand and certificates while parked, then FIFO decode publishes held-store indexes and releases source scratch without a Memory capability. Interrupted settlement forgets those stores before reuse; model, task and organisation guard comparisons still select native reads during acquisition.

Allocation diagnostics lazily cache retained sprite heap totals until a row is
replaced. The total includes unused retained output rows; resets, scalar
organisation updates and appended empty rows preserve those same child heaps.

Run `cargo test -p collector-ship-models` and `cargo clippy -p collector-ship-models --all-targets -- -D warnings` for its synthetic checks.

The existing medal-modifier rebuild hook also forwards a dirty notification to the shared division-history domain; it installs no second patch and reads no extra native state.

The ShipModels critical-damage nonempty predicate forwards the existing held organisation header under the same ship certificate, without another native read or hook.

Non-mega carrier hosts carry their Air capacity subtype under an independent refit certificate, forwarded by the existing refit patch. First/changed hosts alone read its retained module predicate; ship experience and equipment-stat writers leave this certificate alone. The worker bulk-reserves reusable storage. Unavailable reads carry an unknown until their certificate or epoch changes; the synchronous fallback acquires none of this suffix. State and mega subtypes derive from existing data.

ShipModels carries an independent raw selected-design naval dominance packet. Only the selected variant demands the 32-byte mutable suffix; original gathered classifier flags select one carrier-size word, and positive nonselected variants add no power reads. Larger uncached pools read the original completed selected source alone, including its otherwise unretained classifier word (40 bytes total). Its mutable words join the existing worker variant comparison; its definition weight and inherited ship category use the equipment-content reload and world lifetime certificate. Losing certification invalidates held packet values and records a named independent refusal. Already-unknown power preserves the core model cache; variant companions require a valid certificate. Synchronous acquisition adds no reads.

Task-force modifier arrays retain shared immutable ownership instead of copying through two hourly arenas. Certified task lookups borrow those arrays directly; sampled task forces still use the fresh snapshot reader and compare its complete observation. Equal fresh arrays keep their allocation. The held ship-pointer index also survives an unchanged census order; new, deleted or reordered held ships rebuild it. Current task selectors, weather/night, fleet/HQ terms, variants and the existing ship-row verification are unchanged.
