# Separate playhead preparation and consumption measurements

The log lane bench prepares synthetic recordings in memory and measures selected append, forget, seek, held-state, and demand-row paths. It fetches each step's parts outside the timed walk. The fixtures include lone forgets, forgotten runs and quiet intervals to distinguish costs. One timing variant reads the same parts directly to separate cursor work from lane bookkeeping.

The recording benchmark can open a whole recording, one run, or a span with prerequisite restores. `check` compares lanes and mirrors against direct rebuilds and compares sampled seeks with steps. `check --passing` compares passing rows against an hourly reference. `time` isolates table stepping, while `seek` separates restore and replay work. `play` drives a consumer frame loop and measures preparation lag, stalls and rebuild swaps. The optional second reader tests changes in selected columns and tables during playing, paused, and sought states. `join` measures the latency for a second reader to receive its initial reset. The consumer moves retired mirrors to a dropper so freeing them is outside its measured apply path.

[Log lane benchmark](../../crates/playhead/src/bin/log-lane-bench.rs) · [Recording benchmark](../../crates/playhead/src/bin/playhead-bench.rs)
