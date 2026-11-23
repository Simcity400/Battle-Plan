# 1396. Viewer counter crates

## Decision

`viewer-counter-kit` owns the layouts, drawing inputs, reports and shared data used by the counter passes.
`viewer-counters-naval` keeps naval windows, base windows and air counters together because they share preparation and drawing code.
`viewer-counters-land` owns land and railway gun stacks and uses the naval crate when task forces take part in the same counter columns.
`viewer-counters` loads the shared assets and recorded content, builds each hour’s tables and coordinates the passes and their retained state.
Layer readers import their row and product types directly from the crate that owns them.
