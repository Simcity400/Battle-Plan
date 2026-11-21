# Record display values at their printed precision

Fixed-width snapshots and zigzag deltas use integer step counts for values whose off-screen digits drift. This retains the existing column layouts and reduces compressed sizes. Playback expands a step count back to native units. Recordings before `DISPLAY_PRECISION_FORMAT` already store native units, so their decoders skip expansion.

Capture rounds before comparing rows, so sub-display changes don't create deltas. Every encoder also rounds before storing step counts, including callers that skip capture. The rounding is idempotent. Fixed5 truncation drops discarded digits of the magnitude. Integer-style rounding chooses the nearest multiple with ties away from zero. The nonzero variants keep one step with the original sign if rounding would produce zero, preserving sign-dependent comparisons and colors. Three extreme signed words are reserved for native and viewer sentinels. Step scaling preserves them verbatim and a malformed product outside the signed range fails the decode.

Only round the elements a consumer shows at the matching precision. Identifiers, counters, dates, seeds, route increments, full-precision values and inputs to exact decisions keep their original digits. The owner accepted that a derived display can differ from the game's last printed digit near a boundary. The encoder finds the magnitude with unsigned arithmetic. Excluding the sentinel values bounds the rounded intermediate within `u64` and keeps wide division out of the common path.

[Display steps](../../crates/replay-rows/src/display.rs) · [Row encoding](../../crates/replay-rows/src/rows.rs)
