# Paint followed values without replacing widget structure

Give each followed query a canonical key and a page-wide ID. Share a store by client, source revision, playback clock and track offset. Pausing can invalidate a prepared step even without a page seek, so discard queued structure at that boundary. A pinned exact seed establishes only its own panel hour. For that hour, use a followed value when its coverage gives the same answer.

The decoder alone owns the mutable body. It can transfer an independent snapshot while applying the next delta. Body kind 1 includes row keys, cell values and states, and the row names and reasons. A delta cannot be applied before a complete body is known. Keep the wire-cell grouping distinct from structure replies with the high bit of the followed ID.

The `live` section places formulas over grouped recorded sources into the rows and columns the structure renders. Formula metadata is flattened by row, kind and source count, followed by each group's source indices. Validate that every source exists, memoize validation per reply object, and let one resolver read either exact row-major values or snapshots pushed under group IDs. A group's acknowledged state tracks the latest message through which its values are current. Missing states retain their reason. Only a missing row of a table whose contract is sparse can establish a zero value; a missing input or zero divisor remains unavailable.

The frame reads and paints live values from the answer's version. A widget renders for status and structural changes. During a change to a set of followed queries, keep the previous fully answered set only while the new set is still waiting and belongs to the same owner and timeline. Switch as a set after all new answers land. Refusal, interruption or another owner releases the previous set. Browser measurements prepare test data outside the paint pass and report the median, reducing the contribution of fixture setup and garbage collection.

[Follow pipeline](../../ui/src/dashboard/follow/pipeline.ts) · [Follow messages](../../ui/src/dashboard/follow/message.ts) · [Live cells](../../ui/src/dashboard/follow/liveCells.ts)
