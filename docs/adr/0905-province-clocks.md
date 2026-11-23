# Replay province clocks with native arithmetic

The province tooltip, combat and air calculations share the game's local hour and night factor. Reconstruct their hourly update from the recorded content stack's typed `NGraphics` defines, integer province centroid and map dimensions. Defining longitude subtracts the GMT column in signed 32-bit arithmetic before widening. Preserve the game's signed wrapping products, fixed-point truncation and reserved division result, including a zero-width feather. The seasonal curve uses June and December turns and the native polynomial constants. The phase within a date selects one of the initialized solar vectors, while the date supplies the seasonal direction. Refuse a negative hour-index residue so it cannot index outside the vectors.

A recorded hour does not by itself prove that every province clock was updated. Both suspension gates must be open, slot zero is never updated, and newly allocated records keep hour zero and night factor 100000 until the first update. Preserve those held values in `DayNightClock`. An updated clock for a centroid inside the map uses 1-based local hours and clamps the night factor to zero through one in fixed5. The local-time tooltip follows the game's signed half-away rounding before choosing the icon frame.

Air fuel forecasts need all 24 factors under a single held seasonal direction. Reuse the initialized vectors independently of the most recent clock update or the mission's selected day/night period.

[Province clock calculation](../../crates/replay-rows-world/src/day_night.rs) · [Recorded clock states](../../crates/replay-rows-weather/src/naval_weather.rs)
