# Capture special-facility flags from program provinces

Program loading and addition set the special-facility icon flag in the game's province graphics. Per-province and dynamic-country resets can clear it. Enumerate the country and program rosters to find candidate provinces, then read the flag from the graphical map attached to the in-game idler. Emit each flagged occurrence in roster order, including duplicates when programs share a province. Capacity accounting includes each occurrence because each one can hold an observed flag.

Province zero denotes the null province and draws no icon. Check bounds explicitly because the native lookup can accept an out-of-bounds province. The graphical map must be present for the table to be available. The traversal uses caller-owned storage.

[Special-facility flags](../../crates/engine-projects/src/special_facility_flags.rs)
