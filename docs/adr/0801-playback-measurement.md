# Measure presented playback from one time window

Opt-in desktop playback logging puts host and page records into a shared wall-time domain with separate viewer outputs. A writer thread keeps disk waits off page and service threads. Its queue bounds bytes and counts lost records. Lost records keep their reported time range so summary windows can account for overlap. A lost page loss-record still contributes the count it reported. Each run uses its own viewer output set and waits for the page and relevant viewer outputs to drain before summarizing. Exit uses the ordered flush when possible and writes a fallback summary if that path did not run.

The summary measures the last start-to-stop or start-to-end window. Play, pause, speed changes, and seeking divide time into playing intervals. A viewer contributes only when active, and its acknowledged frames must belong to the same seek and show progress since it became active. An unchanged acknowledged position leaves a stall open. Uncovered playing time contributes zero progress, and overlapping record losses or an unanswered final page flush identify incomplete evidence. Page-frame lateness counts whether the preceding frame had already been rendered within a refresh to distinguish loss after the main thread from a missed main-thread deadline.

[Desktop performance log](../../crates/desktop/src/perf.rs)
