# Preserve recorded table state when writing columnar frames

The columnar frame writer has one table writer and one dictionary per continuous run. It does not reuse another run's state. Preparation and commit are separate so the manifest, sections, appended strings and held table state advance together. An hour interns strings during preparation. Discarding that hour could leave dictionary IDs in use without their appends, so another preparation is refused while it is pending and a failed append that leaves partial dictionary state poisons the writer.

## What a reader sees

A complete table sampled this hour gets this hour's sample timestamp. An unchanged unread table carries its previous sample time, including when the row encoder repeats its rows in a keyframe. In that case a row decoder newly starting at the keyframe cannot recover the previous time. An unread change, a truncated read, a refusal and a table outside the capture mode each carry no sample time. Omitted tables also carry no read bit even if the producer timed them.

For live captures, a staggered table can carry an off-slot refusal only if the run already has a complete view. A truncated live read cannot seed a keyframe. During conversion, the source recording supplies both status and sample time, including at an intermediate cut. A recorded truncated view keeps its captured rows and missing count. A recorded refusal is never filled with previously held data because the source already applied its carry rules. A complete result's delta is usable only when it describes the immediately preceding complete view in this run. The first hour or an hour following unavailable or partial state compares whole. A changed key without a corresponding source row is refused.

## Preparing the bytes

The writer borrows the row encoder's baselines and changed rows only when both have committed the same preceding branch and hour. A comparison that guarantees unchanged membership aligns its changed indices with those baselines. Shared row allocations stay alive through commit so pointer equality proves unchanged rows without an additional row comparison. An owner slice replaces only the named owners' rows, with their held keys kept in ascending order. Keys for revoked owners are removed even when a replacement slice supplies no rows.

When complete rows arrive in key order, sparse inserts, changes and removals can be staged against the held rows. Out-of-order rows use a full write. The first observation of changes in more than three quarters of the rows also postpones diffing for 24 hours. While row positions match the baseline, staging each dense row beside its previous row still lets unchanged opaque cells stay held. An insertion or removal that breaks this alignment uses the full rows.

Preparation and commit can each give tables to multiple lanes. The order is by last observed cost, with the calling lane completing anything helpers leave when the game resumes. Workers use read-only dictionary access during preparation. A result that needs a new string or failed on a worker is prepared again in table order on the calling lane so string IDs, restarts and bytes match the sequential path. Restart state includes each table's dictionary references and appends since the last restart. Work on an unchanged table begins no column work where it stages no row, and an unchanged whole table can leave its held columns alone. The full sweep and row-staging modes check that these savings still produce the same wire bytes.

The parity tests feed the real recorder store and row encoder the same conversion inputs, including truncation, missing tables, sample retention, owner slices, and helper lanes stopping halfway through a step. They compare the wire and read-back state. Live error paths use allocation-free error kinds even inside a diagnostic scope, while offline writers retain messages.

[Frame writer](../../crates/replay-journal/src/native_frame.rs) · [Parity tests](../../crates/replay-journal/src/native_frame/tests.rs)
