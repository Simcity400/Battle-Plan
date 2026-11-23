# Carry the verified fact stream inside the recording

Keep each recorded layer's verified non-art facts, its file list and replacement paths in the recording's fact manifest. Recheck digests when writing. Read each distinct digest once, in first-use order, cut one stream into independently compressed chunks and reuse compressed chunks by content. Keep a layer's named withholding reason when its content cannot be proven. Art follows the pack's versioned selection rule and is read from the installed layer where available.

Carry only the recorded language when an entire localisation file belongs to another language, unless another layer supplies a carried file at that path. Leaving out a higher layer's file must never uncover a lower one. Record executable tables with the pack and put those tables in scope for its readers, so no playback operation depends on the executable currently installed.

Compute equipment needs per archetype from the pack's own facts and executable tables, in stable key order. This lets playback consume the derived fact without loading the equipment database. Packs predating the field remain readable, and unavailable or unsupported versions name why the equipment fact is missing. Reject truncated or extended sections instead of treating a partial decode as fewer entries.

A fact can be streamed one chunk at a time for memory-bounded readers. Finish verifying its digest after delivery and invalidate every earlier piece if verification fails. Preserve the distinct states of a missing file, a withheld layer, uncovered recorded facts, and bytes that happen to be installed today.

[Fact manifest and stream](../../crates/content-reader-layers/src/content/facts.rs) · [Equipment fact](../../crates/content-reader-key-names/src/equipment_facts.rs)
