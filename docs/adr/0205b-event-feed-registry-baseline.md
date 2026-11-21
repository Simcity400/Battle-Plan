# Establish the event feed registry baseline under a stable observation cut

At the first eligible boundary of a native observation segment, the event feed samples the retained script registry. Later changes come from writer notifications. It does this once per segment so an active registry is not polled every hour.

The sampled state must stay within a stable cut. A writer in flight, a writer that crosses the sample, or a recorded decline that crosses it leaves the attempt uncommitted so the next boundary can retry. Even a read error is classified as a crossed sample when the cut has failed. A fault under a stable cut has a named refusal and settles the segment's attempt.

[Registry baseline](../../crates/collector-observers/src/event_feed_history/registry.rs)
