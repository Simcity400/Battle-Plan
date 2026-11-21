# Separate recorder probes from capture

The tray app uses the recorder's in-process API and the command line uses the same capture lifecycle. The read-only probe identifies the running campaign and its load signal before attachment. A new load of the same campaign changes the signal. The menu, a load in progress, an unreadable clock, and an exited process can all yield no campaign. Two agreeing reads are required for identity and clock observations. The smoke clock also requires agreement on pause state and samples the camera from the second read.

The clock can exist during startup. The loaded indication also requires the in-game idler. Pause and camera are withheld when their sites cannot be verified, with the causes named together or by presentation field where the API reports them. An unrecognized executable build remains eligible once the necessary layouts verify against the located image. Location is reused per executable and loaded base, while a withheld campaign probe stays withheld for that process's lifetime. These probes neither attach the collector nor provide recorded data.

The ship display cache witness is a separate read-only diagnostic. The operator holds a paused state through the matching recording boundary. The witness includes observed build and content facts without claiming a collector receipt. It preserves existing evidence through exclusive output creation. A recorder restart alone contributes no damage to the diagnostic.

[Tray controller](../../crates/recorder/src/controller.rs) · [Smoke clock](../../crates/recorder/src/clock.rs) · [Located sites](../../crates/recorder/src/game_sites.rs) · [Ship name witness](../../crates/recorder/src/ship_name_witness.rs)
