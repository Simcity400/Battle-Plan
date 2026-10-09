# Raid tally

## What it shows

Observed launched/succeeded raids per native country, as a race or lines.

## Products it reads

[`raids`](../../../../crates/dashboard/src/products/raids.rs) shares the [Raid log](../raid-log/README.md) scan and availability check; the shared race kit supplies dated country looks.

## Settings

Top 10 by default, manual/groups; campaign/follow/pin/range, race/lines, launched/succeeded/both, date/end labels and shared playback/axes.

## Notable behaviour

- Launch requires observed in-progress work or flight distance. Terminal outcomes alone do not prove launch; success includes native success/critical success, excluding limited success.
- Success may exceed proven launches and is never clamped. Both ranks by launched, retaining success-only countries; the inner success segment may extend beyond the launch bar.
- Counts are witnessed-event lower bounds, marked + for unobserved earlier/between-read raids. Proven zero remains zero; missing reads are unknown and break runs.
- Counts follow slot/id holders through renames, excluding unrelated tag reuse. Groups preserve membership dates/gaps; transitions appear only at supplying reads.

## Files

| File | Responsibility |
|---|---|
| [RaidTallyWidget.tsx](RaidTallyWidget.tsx) | Shared race/lines and dual measures. |
| [raidTallyData.ts](raidTallyData.ts) | Validation, native identities, counts and tooltips; adjacent tests. |
| [definition.ts](definition.ts), [index.ts](index.ts) | Availability, options and registration. |
