# 0906. Record map events independently of a playback viewpoint

The collector observes events at the decision point before the game's local-viewpoint filter. A recording can therefore evaluate which events the chosen country would have seen. Scripted map refreshes are observed after the selected mode and script limit checks pass. A successful refresh identifies the authored mode by declaration ordinal. Critical shore bombardments retain both the floating text and the explosion for every viewpoint.

The single row for an hour carries the events in global publication order and a coverage record for each family. An event is evidence it happened even if its family's coverage is incomplete. Complete coverage of a family with an empty event list certifies no events. Incomplete coverage certifies no absence. The health bits distinguish an absent hook, a disabled interval, observation starting during an interval, refused reads, capacity, native unwind, output loss and publication still in flight. Missed counts accumulate from the start of each observation epoch.

An interval runs between hour reads and may include events stamped with either endpoint. Its publication range starts at `first_sequence` and extends up to `next_sequence`. Continuity requires the preceding interval's `next_sequence` and the same epoch. Attachment, world reset and load change the epoch. A late in-flight event can appear in a later interval with an earlier sequence. Family faults follow the wire bit assignments and unknown fault bits are malformed.

The display operands retain the simulation's raw country references. Unit intel samples keep canonical country ordinals, signed levels, source order and repetitions. For fake divisions and railway guns, preserve both the displayed province and the province used for visibility. Captured generals retain the two country refs. Paradrops retain landing losses and fog, while resistance retains the best victory-point province, the controller and the owner's overlord. Damage notifications emit one event per affected building level. The recorded reason selects which text to show.

[Map event rows](../../crates/replay-rows-events/src/map_event_rows.rs)
