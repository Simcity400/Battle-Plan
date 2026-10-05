# Raid tally

Observed raid launches or successes per country, using the same cached `raids`
branch scan as Raid log. Race is the default; lines is a setting. The default is
the top 10 over every existing recorded country's nonzero count; chosen existing
countries keep their recorded zero, and smart groups work. A complete empty read
shows **No raids launched yet.** (or **succeeded**); missing reads stay unknown.
The shared `CountryRaceView` supplies style-1 bars/lines, country colours, names
and flags at the hovered hour, gaps, settings and right-click Jump to.

**Observed launches** requires a read in native in-progress phase or nonzero
recorded flight distance (`140FDA5A0`). Terminal outcomes alone cannot prove
launch: the console can finish a raid directly without
launching it (`1402574C0`). **Observed successes** requires the native success or
critical-success outcome; limited success is excluded (`140FD7600`). Success can
therefore exceed proven launches; it is never clamped. Dates enter by the read
that proves the transition, not an interpolated time between reads.

Totals count witnessed events. A **+** labels a lower bound because pre-recording
or entirely between-read raids are not enumerated. Missing country rows supply no
value, never zero; a chosen country's recorded zero stays zero. Gaps break runs, and a different
country taking a tag resets its count and identity. The product ranks through
`crates/dashboard/src/ranking.rs` and refuses oversized replies instead of truncating.

Choices where the spec is silent: a **Count** setting selects launches (default)
or successes; one country line per selected count; no one-country mix; the race
kit's optional date overlay and line-end labels/playback/axes settings; shared
top-10 default. Picker availability is the same content/recording check as Raid
log. Browser fidelity and real recording checks remain unverified.
