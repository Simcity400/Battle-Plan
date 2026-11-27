# Power spend summary logs (format 155)

## Context

In formats 143–154, answering the political or command power spending breakdown or markers required scanning three multi-block state tables across the full campaign span:

- `PowerSpendTotals` (cumulative 32-column state table checkpointed across blocks) for one-off payments, daily gains, unmatched costs and unknown daily credits.
- `CountryLinks` and `CountryPowerChanges` (full raw event history) for instant gains and command-power allocation/refund cap losses.
- `PowerSpendDays` (per-hour log table introduced at format 143) for recurring daily costs and daily credits.

On a 10-year vanilla recording, scanning `CountryPowerChanges` and `PowerSpendTotals` dominated the first reply's CPU cost (keeping the first spending reply above 1 second even after reading `PowerItems` once at the end of the span).

## Decision

Format 155 introduces two derived per-hour log tables emitted by `PowerLedger` (writer version 5) and retires `PowerSpendTotals`:

- `PowerSpendPayments` (table 527, 104-bit key `holder:32 | hour:32 | currency:8 | entry:32`, `Output::Log(_, Keys::Hour(40))`): records each holder's one-off payments (`payments > 0`) and unknown daily credits (`payments == 0 && unknown > 0`) per currency and item ordinal at the hour they are read, with `ordinal`, `known`, `payments`, `unknown`, `uncovered`, `latest_hour`, `latest_sequence` and `latest_ordinal`.
- `PowerSpendGains` (table 528, 104-bit key `holder:32 | hour:32 | currency:8 | entry:32`, `Output::Log(_, Keys::Hour(40))`): records each holder's daily gains and unmatched daily costs (`event_hour = None`) and instant gains and command-power cap losses (`event_hour = Some(row.hour)`) per currency at the hour they are read, with `event_hour`, `gained`, `capped`, `days`, `unknown_days` and `unmatched`.
- `PowerSpendTotals` (table 475) is retired at format 155. Format 155 recordings stop writing it so the recording carries one representation rather than two; older recordings (< 155) continue reading `PowerSpendTotals` alongside `CountryLinks` and `CountryPowerChanges` without change.
- `PowerItems` continues assigning ordinals to every non-skipped power change so item identities (`holder:ordinal:first_read`) stay unchanged.
- Status propagation from `CountryPowerChanges` applies to `PowerDayParts`, `PowerSpendDays`, `PowerSpendPayments` and `PowerSpendGains`. Status propagation from `CountryPowerDailyCosts` applies only to `PowerDayParts` and `PowerSpendDays` so missing daily cost rows do not drop one-off payments or gains at that hour.

## Consequences

- Format 155 spending queries read only `PowerSpendCoverage`, `PowerSpendDays`, `PowerSpendPayments`, `PowerSpendGains`, a single end-of-span `PowerItems` snapshot, and status/read metadata for `CountryPowerChanges` and `CountryLinks` (no `PowerSpendTotals`, `CountryPowerChanges`, or `CountryLinks` row walks).
- Every key field (`holder: u32`, `hour: i32`, `currency: u8`, `entry: u32`) and per-hour writer loop scales directly with the countries and events present in the recording without vanilla caps, and currency codes are validated through `PowerCurrency::try_from`.
- Recordings before format 155 open and play identically via the existing `PowerSpendTotals` + `CountryPowerChanges` reader path.

[Power ledger writer](../../crates/recorder-derive-politics/src/power_ledger.rs) · [Power ledger reader](../../crates/dashboard-politics/src/products/power_ledger.rs)
