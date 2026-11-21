# Name a recording from its own hours

Both recorders use the same naming rules. The name uses the first and last recorded in-game years and the local player's country at the latest hour. If several branches reach that hour, the latest written wins. Appended spool hours win the same tie. A name requires recorded operands for the country and any overlord. Localisation is taken in the recording's language. A refusal falls back to the recorded tag where available, and an unnamed file gets the generic campaign stem. The stem is sanitized for Windows before the year and multiplayer suffix are assembled.

One observed hour with two players proves multiplayer, even if the roster is truncated. A missing roster cannot prove single-player. A predicted name waits until both the roster and country evidence are conclusive. The recorder can combine already applied hours with the written recording without reading back those hours before publication. It reuses a roster result only when the recording still contains the previously scanned revision and blocks.

Size inspection is offline. It attributes stored bytes to blocks, restarts, summary pieces, index structures and game facts and includes headers, footers and superseded structures in the remaining accounting.

[Recording names](../../crates/recorder-replay/src/recording_name.rs) · [Roster reuse](../../crates/recorder-replay/src/replay_io.rs) · [Size inspection](../../crates/recorder-sizes/src/sizes.rs)
