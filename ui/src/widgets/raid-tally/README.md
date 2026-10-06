# Raid tally

Observed raid launches and successes per country, using the same cached `raids`
branch scan as Raid log. Race is the default; lines is a setting. Count offers
**Launched**, **Succeeded** or **Both** (default). The default country selection is
the top 10 over every existing recorded country's nonzero count; chosen existing
countries keep their recorded zero, and smart groups work. A complete empty read
shows **No raids observed yet.** (or the selected count); missing reads stay unknown.
The shared `CountryRaceView` supplies style-1 bars/lines, country colours, gaps,
settings and right-click Jump to. Tooltip names and flags use the hovered hour. Smooth
monotone lines and easing bars use the economy widgets' shared path; numbers and
read dates stay exact, and missing reads stop easing.

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

In Both, launched ranks the race. The shared kit draws succeeded as a thinner
inner segment in the country colour, inside a faint launched bar. Success-only
countries remain eligible; a success exceeding proven launches extends beyond
the outer bar, with both exact counts labelled and the axis covering both. Lines
use solid launched and the shared secondary dash for succeeded in the same
country colour, with measure-labelled end labels and both values in tooltips.
Country choices use the shared selector without a smaller widget-specific limit.

Choices where the spec is silent: Both ranks by launched; overflow extends the
inner segment; solid/dashed secondary lines; no one-country mix; the race kit's
optional date overlay and line-end labels/playback/axes settings; shared top-10
default. Picker availability is the same content/recording check as Raid log.
Browser fidelity and real recording checks remain unverified.

Drawn names and flags use the existing sampled `useCountryLooks` path: one grid
hour per country, bounded by its recorded stretch and identity. Pending looks may
retain the prior sample within that segment; completed missing reads use the
shared fallback. Tooltips request country labels at the hovered hour.
