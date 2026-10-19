# recorder-replay

Replay recovery, admission records, finalization, append and inspection. replay_io.rs owns load refusal/omission values and atomic JSON publication shared with capture. Recovery validates the committed prefix and terminal evidence; tray sessions publish only admitted loads. Capture lifecycle is owned by recorder.

- [replay_io.rs](src/replay_io.rs)

Shared development and content rules live in [AGENTS.md](../../AGENTS.md). Capture and fresh-save checks are described in the [recorder guide](../recorder/README.md).

Inspection reduces script-event registry operations at each branch's final recorded hour. It prints pending instances and due hours, fired membership, country delayed timers and captured scope operands, with named unavailable states when coverage cannot certify them.
