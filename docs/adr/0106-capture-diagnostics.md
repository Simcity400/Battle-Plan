# Keep callback and spool timing separate

The callback histogram measures recorded hours using entry time plus callback time. An hour blocking the game thread for more than 50 ms includes the most expensive tables and phases in status. The sequence counts captures since attach before that hour. Variants drain statistics aggregate wall time and the counts of designs read, unchanged designs, walked entries, and source notes.

The spool independently records work above 100 ms. Its hour is absent for inter-frame work such as sizing, flush or status. QPC deltas report the most expensive steps on the spool and on its helpers. The optional cycle sample distinguishes CPU work from wall time spent blocked. The conversion breakdown is available only for a frame. Cycle lookup failure leaves the sample absent.

Spool phases normally wait out the game’s hourly tick before starting. The idle wait and phases explicitly admitted inside the tick bypass that gate. Gate wait has its own final timing slot, so it remains visible without inflating an operation’s elapsed wall time. Test-only counters record which thread starts each phase.

[Slow-hours status](../../crates/collector-spool/src/slow_hours.rs) · [Phase accounting](../../crates/collector-workers/src/spool_phases.rs)
