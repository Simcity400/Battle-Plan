# Make map events from their own hour

`MapEvents` is an hourly replacement row. The frame keeps its own copy and applies each step to it because the shared mirrors already hold the last step of the frame. Each interval is presented once and in order. A step covering several hours can only present its last hour's interval. The scene sees the skipped hours as a discontinuity.

Intervals with events use a worker with its own stepped copy of the context and visibility tables. An event uses the viewing country, naming data, and province fog of the interval's hour. The worker steps only within the seek epoch and as far as the frame has handed over. A reset supersedes earlier requests. This prevents a seek back or several steps in one frame from reading a future country's events or vision. The worker ends when the layer is dropped.

An empty interval or a named refusal is made immediately. Late table declarations hold their interval until a subsequent step brings the rows. The worker polls for that step during a composed settle and can use the requested hour's context when a declaration lands past a held head. Ordinary settling leaves overdue intervals in order for a later frame. A whole settle fails by name if the interval cannot be made. It does not make an unavailable history look empty.

The proof compares the head's interval with the independently reconstructed interval and with a fresh compose. The tests exercise a country switch, changing province sight, late tables, multiple steps per frame, and seek supersession. [Layer](../../crates/viewer-layers-political/src/map_text.rs) · [Worker](../../crates/viewer-layers-political/src/map_text/worker.rs) · [Tests](../../crates/viewer-layers-political/src/map_text/tests.rs)
