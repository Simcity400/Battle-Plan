# 1115. Evaluate scripted map products off the frame thread

The viewer evaluates raid offers and the viewing country's music chances against decoded rows at the playhead. Its worker reads the recorded trigger inputs and executable metadata. The frame offers the newest eligible rows during apply and joins the result during settle. It passes replaced row and product references to the worker to drop, since releasing the last reference can free megabytes of rows. Closing the reader may join the view store worker, so that also happens off the frame thread.

Raid icons need publication-rate evaluation. The decoded rows are produced by the same step that requests them, which normally puts the icons one publication behind. That interval counts as current for the step. Greater lateness reports the actual hour. The cycle carries work across evaluations and restarts on seek, perspective change, content replacement or time reversal. A whole settle runs the offer cycle to completion and requires music evaluated at its hour if music is enabled. A batch that advances nothing ends the round, including a mod configured with zero batch size. The layer can wait for evaluation of available rows, but waits for a subsequent step to deliver rows still being built. It reports a whole settle timeout with the missing hour.

Music has no corresponding drawing and tracks are chosen minutes apart. Ordinary playback evaluates chances at most four times a second, memoized by identity. Tests step a separate clock. Jobs and results are stamped with a seek and perspective epoch. A panic abandons the failed seek and its cycle, leaving later seeks able to start afresh.

Scripts can read tables outside the declared trigger core. Undeclared reads accumulate table and field masks and suppress that evaluation's product until those inputs are ready. Learned inputs survive closure. Tools that hold the playhead still prime inputs upfront because newly learned lanes only join after the held head. The proof compares the complete product with the direct evaluation of the same hour, records when tables are still arriving, and measures icon age in hours and step costs with and without the scripts layer.

Timing reports accumulate histogram buckets and event counts so diagnostics do not retain a sample for every hour played.

[Scripted layer](../../crates/viewer-layers-political/src/scripts.rs) · [Proof](../../crates/viewer-playback/src/proofs/scripts.rs) · [Tests](../../crates/viewer-layers-political/src/scripts/tests.rs)
