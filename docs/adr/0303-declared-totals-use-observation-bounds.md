# 0303. Keep total windows within one observation

A running total's window is the difference between the total as of a read and the total at a read exactly one window before it. Both ends must be in the same holder's stretch and in the same observation. Neither a missing earlier reading nor a totals restart supplies an assumed baseline. A request that starts after the observation began looks back to the holder's stretch and widens its read when needed to determine the window start. The clock keeps recorded read hours and the starts of observations even if an observation began before the requested chart span.

Parts resolve within a run because the same content can have different IDs in another run. A set adds its selected parts. A part with no row can use the declared rest row, and an unreadable cell instead names its part and recorded reason. A total or difference outside the signed 64-bit range is reported as unavailable. Runs mark a later identity segment's first hour as an opening so chart assembly can preserve that boundary.

[Running totals](../../crates/dashboard-query/src/series/totals.rs)
