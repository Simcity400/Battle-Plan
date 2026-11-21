# 0807. Compare weather periods in the game's non-leap calendar

The content decoder and replay use one weather-period calendar to avoid disagreement at month boundaries. Weather periods use the game's non-leap year. Bounds begin at the first hour of year one and are expressed as zero-based month and day components internally. Normalize to year one before applying each bound's month and day. A month adjustment preserves the day offset. A day beyond a month's length rolls across the boundary, so February's overflow can land in March. Month reads beyond the native table are refused, while the sign and wrap behavior of valid or unusual inputs is preserved within 32-bit arithmetic.

Lookup compares the observed hour's equivalent day in year one at hour zero. The two bounds are inclusive. When periods overlap, the first period in content order wins. Temperature defaults are supplied only when that period's content omits a range.

[Weather period arithmetic](../../crates/weather-period/src/lib.rs) · [Boundary tests](../../crates/weather-period/src/tests.rs)
