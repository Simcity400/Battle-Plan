# dashboard-base

The dashboard's shared product layer, below every product crate and the
[dashboard](../dashboard/README.md) itself: recording sources and their content,
the catalogue loader, the prepared store's shared pages, the product contract and
the shared products other products build on. Nothing here names the product
registry, the preparation pipeline or the service; the dashboard re-exports every
module at its old `dashboard::` path, and product crates name what they use
here as `dashboard_base::…`.

| Module | What it does |
|---|---|
| [source.rs](src/source.rs), [content.rs](src/content.rs), [catalogue.rs](src/catalogue.rs) | One recording revision with its metadata, resolved content and typed per-source caches; content verification and cache keys; one content catalogue per source and folders (the catalogue itself is in [dashboard-text](../dashboard-text/README.md)). |
| [prepared/](src/prepared/mod.rs) | The prepared store of one recording path and its source key, each branch's published coverage, binary series and event batches, frame changes, exact group sums and progressive (prepared plus recorded tail) query helpers. |
| [prepared_chart.rs](src/prepared_chart.rs), [prepared_measure.rs](src/prepared_measure.rs), [prepared_production.rs](src/prepared_production.rs) | Aligned width summaries and event clusters; exact balance and holder observations; production observations and selections. |
| [prepared_map.rs](src/prepared_map.rs) | Sparse native map checkpoints prepared beside the products, and the reader that serves viewer playback from them. |
| [ranking.rs](src/ranking.rs) | Event-based top-N selection and clipping. |
| [read_runs.rs](src/read_runs.rs) | Read-derived value runs and known coverage. |
| [research_dto.rs](src/research_dto.rs) | The research products' reply types, shared with the frontend. |
| [products/product.rs](src/products/product.rs) | The `Product` trait, `ReplyWarnings` and the exported `reply_warnings!` macro, the shared parameter types and validators, and the reply budget check. |
| [products/](src/products/mod.rs) | The shared products: country groups and history, country measure, experience and factories, equipment, equipment icons, produced equipment and production spending, faction totals, force census, territory and its breakdown, politics content, idea history and format, character portraits and production lines; and the small shared modules several product families name: artwork (focus, technology and equipment art with its research and resource titles), research, resources, place names, country styles, battles and fixed-point runs. |
| [test_fixtures.rs](src/test_fixtures.rs) | Test sources and content options over dashboard-core's recording builders. |

The `test-support` feature compiles the fixtures, test oracles (the `#[cfg(test)]`
whole-recording loaders of prepared-only products) and the test instrumentation
(read and work counters) for the tests of the crates above, so their test builds
behave exactly as this crate's own. Only dev-dependencies enable it.

## Prepared map checkpoints

[prepared_map.rs](src/prepared_map.rs) prepares the map's native inputs and shared
authored-trigger tables. Independent native checkpoints retain decoded rows,
partial censuses, unavailable fields and supplying hours every 720 accepted
updates. Each existing Store range uses zstd9 with a checksum. Hourly updates
remain in the recording; no prepared update pages or packet seeds are written.
Full-selection `map.resume` state is replaced independently for all preparation
observers. Progress keeps checkpoint ordinals per branch and the accepted source
cursor. Appended preparation restores that native resume state.

The shared [Reader](src/prepared_map/reader.rs) serves viewer playback and
acceptance. It binary-searches checkpoint headers, restores the nearest native
state and calls the ordinary selected recording reader through the target hour.
Source parsing starts at each source block's keyframe; native application skips
the frames preceding the checkpoint cursor. Selected table decoding uses half
the available processors, at least one, with the existing below-normal worker
priority. Consecutive steps mutate the native decoder in place. A step needing a
source load reads the rest of that accepted block once; later steps retain it.
Only one parsed source block remains between requests. Source admission precedes
passed-hour callbacks, and reads never extend beyond accepted branch progress.
No recording format, directory, index or setting is added.
Changed gaps become interval records with reasons in original occurrence order,
including repeats and overlaps. Readers binary-search gap records on disk.
