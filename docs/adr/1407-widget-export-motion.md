# ADR 1407: Widget Export Motion (Ease-After-Change)

## Status

Accepted

## Context

Offline video export steps dashboard widgets and whole pages at a constant frame rate (30 or 60 fps) across recorded campaign hours. Live dashboard widgets project strictly from a bounded view at the current hour `H` (`docs/adr/1402-dashboard-family-architecture.md`) and never read future hours (`H + 1`).

During live playback, only row rank positions glide (`bars` and `table`), and `glideDurationForSpeed` disables gliding above 168 hours/second. For video exports, viewers expect smooth bar and meter motion between recorded hours without inventing unrecorded values or breaking the bounded-view contract.

## Decision

1. **Ease-after-change (`prev -> H`) on a single opt-in `LaneBinding.motion` mechanism.**
   - Live playback leaves `binding.motion` unset, preserving live defaults and playback cost unchanged.
   - Offline export provides `binding.motion = { easeValues: true, glideDurationMs, nowMs, speed }` driven by the export frame's deterministic virtual clock (`frameIndex * 1000 / fps`).
   - `glideDurationForSpeed` honors `overrideMs` when supplied so export is not constrained by the live 168 h/s cutoff.

2. **Animated geometry vs. honest recorded figures.**
   - **Bars (`bars`)** ease drawn bar widths/lengths and row rank positions from the previously rendered hour (`before`) to the current hour `H` using cubic ease-out.
   - **Meters (`meters`)** ease drawn track and threshold fill fractions from `before` to `H`.
   - **Tables (`table`)** glide rank-reordered rows using the supplied `glideDurationMs`.
   - **Numeric labels and readouts always show the exact recorded value at `H`.** At high speeds, the drawn bar or meter trails its printed label by up to `glideDurationMs`.
   - **The remaining families (`time-chart`, `timeline`, `matrix`, `cards`, `flow`, `tree`, `deck`) step on recorded hours** without geometric interpolation.

3. **Strict snap boundaries.**
   - Any transition to or from a missing value (`notRecorded` or `notApplicable`), any newly appearing row, and any continuity discontinuity (seek, load boundary, unrecorded gap card, or skip-cut card) snaps immediately to `H` without easing.
   - Zero future reads are introduced; `BoundedReadView` and `check-future-reads.mjs` remain unchanged.

## Consequences

- Widget and page exports remain 100% faithful to recorded campaign hours and preserve the bounded-view invariant.
- Bars and meters move smoothly across sub-hour frames at 30 and 60 fps while keeping numeric figures exact.
- Live playback incurs zero extra work because `binding.motion` is absent outside export presentation mode.
