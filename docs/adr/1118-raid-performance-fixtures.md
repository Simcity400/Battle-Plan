# Exercise raid and combat-scale viewer workload without treating it as game evidence

The viewer tests join synthetic sprite fixtures to the shader owner and build movement maps from test inputs.

The churn oracle compares the kept evaluation hour, icons and report with a serial reference after every batch. It changes each family of inputs, including controller and state membership, and also tests skipped hours, moving rows, removed paths and missing memo pairs in different parts of the same batch. A completed cycle must reproduce the reference after a pause. Reused icons and resumable army searches have their own from-scratch comparisons. Some churn operations are deliberately synthetic and test dependency invalidation even when a live session would keep its memo append-only.

The unit scale example derives appearances and names from a supplied recording. It adds combats at about twice the measured peak share of divisions in combat and uses four divisions per side to approximate the observed combat size. It preserves original combats and their recorded side rows unless an hour changes membership. Synthesized rows use logical and positive roster owners, deduplicated by country link, in traversal order. The fixture withholds raid targets to isolate unit performance. A load signal initializes known engagement flags. Hourly country and division value changes exercise invalidation independently from movement or combat. Selected combats rebuild engagement on removal and rejoin. This provides workload and engaged-index evidence. Native visual and marker fidelity needs its own evidence.

[Raid tests](../../crates/viewer-raids/src/map/raids/tests) · [Unit scale fixture](../../crates/viewer/examples/unit_scale_fixture.rs) · [Viewer dependencies](../../crates/viewer/Cargo.toml)
