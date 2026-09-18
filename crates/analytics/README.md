# analytics

Replay queries without map assets or live game access. The desktop Analytics tab uses this crate to read country history; the native map is a separate consumer of the same recording.

## Query contract

Queries select one branch and an inclusive hour range. Do not merge branches through probable parent links. Observed country snapshots populate the country list; that list is not proof that a country exists in every hour.

The established country-history view includes political power, command power, stability, war support and convoys. Keep missing hours, unavailable or truncated tables, country-absent samples and recorded zero distinct. Gaps must break chart coverage rather than become connecting lines or zeroes.

A history query scans the selected branch's blocks and returns a bounded result, currently at most 1,200 intervals. An aggregated interval retains first/last samples and exact per-metric minima/maxima, not the complete path between them. Narrowing the requested range provides more detail; do not describe an aggregated path as exact hourly history.

Native fixed-point values remain integers. Frontend data uses decimal strings plus a scale; floating point is for chart coordinates, not authoritative values. Carry table sample origins through consumers instead of presenting carried values as freshly acquired.

## Types and integration

Implementation is under [src](src/). Rust owns the generated frontend data types:

```powershell
cargo run -p analytics --bin export-types
cargo run -p analytics --bin export-types -- --check
```

Run these from the repository root. Generation updates `ui/src/generated/analytics.ts`; do not hand-edit that file. Follow with the frontend and workspace checks in [AGENTS.md](../../AGENTS.md).

Queries must honor cancellation between blocks and frames. The desktop host bounds worker concurrency and caches; do not bypass those limits with unbounded parallel scans. Results are tied to a file revision, and navigation to the map must reject a stale result when the recording has changed.

See the [desktop guide](../desktop/README.md) for host ownership and the [frontend guide](../../ui/README.md) for presentation and calendar behavior.
