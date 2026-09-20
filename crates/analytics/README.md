# analytics

Replay queries without map assets or live game access. The native dashboard runtime uses this crate for exact playback windows and country history. Source handles expose the same `Arc<Replay>` to native maps, so views do not need independent copies of the recording.

## Query contract

Queries select one branch and an inclusive hour range. Do not merge branches through probable parent links. Observed country snapshots populate the country list; that list is not proof that a country exists in every hour.

`land_owners.rs` separately discovers countries with recorded state ownership at any hour across the recording's branches. It resolves each state's owner through that hour's CountryLinks and Countries; controllers and cores do not qualify. Positive rows from a truncated state table remain evidence, while missing mappings, unreadable history and lost baselines never create owners. Nonzero owners without a link, country row or tag produce an incomplete-coverage warning while valid witnesses remain available. The result retains branch-specific identity witnesses, is bounded, and checks cancellation between blocks and frames. It scans once for a native source and retains only the compact catalogue, not per-hour ownership arrays. It reconstructs only campaign identity, states, country links and countries; unrelated table baselines are not built. The shared block reader still parses and validates every table update, but immediately discards unrelated payloads instead of retaining them across the block. A data-only bridge check on the existing 1,441-hour recording reduced the uncached discovery request from the original 8.44 s to 2.20 s (previous pass: 4.65 s), with identical 96-country catalogues, ownership witnesses and warnings. The development build also optimizes the native compression codec. The cached request stayed below 1 ms. These development-build measurements exclude source opening and game artwork loading; they are not a general latency guarantee.

The established country-history view includes political power, command power, stability, war support and convoys. Keep missing hours, unavailable or truncated tables, country-absent samples and recorded zero distinct. Gaps must break chart coverage rather than become connecting lines or zeroes.

A history query scans the selected branch's blocks and returns a bounded result, currently at most 1,200 intervals. An aggregated interval retains first/last samples and exact per-metric minima/maxima, not the complete path between them. Narrowing the requested range provides more detail; do not describe an aggregated path as exact hourly history.

Native fixed-point values remain integers. Frontend data uses decimal strings plus a scale; floating point is for chart coordinates, not authoritative values. Carry table sample origins through consumers instead of presenting carried values as freshly acquired.

## Types and integration

Branch metadata includes the recording's explicit gap intervals and reasons for timeline markers. This reads the existing gap index without decompressing history. Reported loss intervals can overlap recovered frames; they are not substituted for the query scanner's actual coverage. Their retained strings and vector allocations count against source metadata admission.

Implementation is under [src](src/). Rust owns the generated frontend data types:

```powershell
cargo run -p analytics --bin export-types
cargo run -p analytics --bin export-types -- --check
```

Run these from the repository root. Generation updates `ui/src/generated/analytics.ts`; do not hand-edit that file. Follow with the frontend and workspace checks in [AGENTS.md](../../AGENTS.md).

`query.rs` defines multi-country history and exact-window demands. `job.rs` advances a scan in bounded quanta. `runtime.rs` schedules current and historical work fairly, shares identical normalized requests and gives each consumer an independently cancellable subscription. `projection.rs` keeps disposable country-table chunks across country selections, overlapping ranges and plot densities. It caches only validated decoded prefixes, retains exact native values and gaps, and never answers an exact query from reduced drawing points. Cache admission is optional: pressure falls back to the authoritative decoder. `source.rs` and `cache.rs` retain byte charges while active handles still own sources or results. Closing a source cannot resurrect it through a late open completion.

Whole-history summaries do not provide exact intermediate samples. Exact-window requests retain one-hour intervals or report a resource limit; they never substitute a coarser result. The present decoded frame API does not expose trustworthy table acquisition origins, so the new query contract reports sample provenance as unknown rather than inferring freshness from a frame hour.

Queries honor cancellation between blocks and frames. Source opening checks it between bounded file reads and catalogue steps. Decompression and parsing of one block remain indivisible. Current limits account for source backing, exposed metadata, admitted country projections and owned result payloads; they are not a bound on total process RSS, parsed replay metadata, every decoder allocation, transport buffers or GPU memory. Completed projection prefixes serve covered overlapping ranges and country sets. Concurrent cold scans or a request extending beyond a cached prefix can still repeat decoding. Runtime statistics expose projection bytes, hits and block reads for measurement. See the [architecture plan](../../docs/analytics-dashboard.md) before extending the cache or scheduler.

See the [desktop guide](../desktop/README.md) for host ownership and the [frontend guide](../../ui/README.md) for presentation and calendar behavior.
