# 0903. Store stepped values at the game's display precision

Stepped columns use the formatter's rounding policy. The table's field declaration owns each top-level step, and a nested value's own declaration owns its steps. Unstepped values, counts, province IDs and sentinel words remain exact. Changes below a step round back to the baseline and write no delta. A legacy format before stepping retains the values and differences in their original units. A stepped field read from a modern column converts back to native units with a checked range.

Ship experience is truncated toward zero to hundredths. This preserves the level and progress the game displays and distinguishes a level threshold from sub-hundredth drift. Supply grace uses the proven share of a positive configured duration for its whole-percent hover. Missing or unusable duration, negative values and values whose share would wrap keep the 0.01-hour representation.

The unit supply footer rounds to hundredths half away from zero. A nonzero amount that would round to zero keeps one smallest printed amount with its sign. It encodes received and origin amounts as shortfalls from their associated requests, which keeps a fully supplied change in demand local to the request words. The origins tag disambiguates the shortfall layout. Older layouts with absolute amounts retain their previous decoding, and older readers reject that tag before reading an incorrect footer.

[Display tests](../../crates/replay-data/src/rows/display_tests.rs) · [Supply footer wire fixtures](../../crates/replay-data/src/rows/supply_footer_tests.rs)
