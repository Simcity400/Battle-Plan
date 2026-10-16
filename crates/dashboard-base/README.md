# dashboard-base

The dashboard's shared product layer, below every product crate and the
[dashboard](../dashboard/README.md) itself: recording sources and their content,
the catalogue loader, the prepared store's shared pages, the product contract and
the shared products other products build on. Nothing here names the product
registry, the preparation pipeline or the service; the dashboard re-exports every
module at its old `dashboard::` path, and product crates re-export this crate at
their root so moved product code keeps its `crate::` paths.

| Module | What it does |
|---|---|
| [source.rs](src/source.rs), [content.rs](src/content.rs), [catalogue.rs](src/catalogue.rs) | One recording revision with its metadata, resolved content and typed per-source caches; content verification and cache keys; one content catalogue per source and folders (the catalogue itself is in [dashboard-text](../dashboard-text/README.md)). |
| [prepared/](src/prepared/mod.rs) | The prepared store of one recording path and its source key, each branch's published coverage, binary series and event batches, frame changes, exact group sums and progressive (prepared plus recorded tail) query helpers. |
| [prepared_chart.rs](src/prepared_chart.rs), [prepared_measure.rs](src/prepared_measure.rs), [prepared_production.rs](src/prepared_production.rs) | Aligned width summaries and event clusters; exact balance and holder observations; production observations and selections. |
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
