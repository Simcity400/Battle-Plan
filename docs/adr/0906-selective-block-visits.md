# 0906. Preserve read evidence in selective block visits

Select watched table fields by their current schema positions, treating a nested value as one field. A key-only watch detects insertion, deletion and availability. Requests can also watch read occurrence, which matters for a staggered table whose newly observed rows equal the preceding rows. Duplicate watches combine their fields. The Campaign table is built and checked each hour for frame identity, even when the query does not watch it.

A block visit reports the first hour and later changes to a watched field, keys, missing count, refusal reason or an observed-read flag. The emitted frame and status have the same meaning as a complete independent block read. Quiet hours retain read metrics, sample hours and evidence of rebuilt versus carried views without reconstructing those rows for the caller. Unwatched fields can change inside a quiet span.

A row delta can also affect a field absent from an older schema. Each reached row carries the appropriate unrecorded field state. Comparing complete view replacements considers both row keys and availability, and conservatively considers a row changed if its fields cannot be compared.

[Selective block reads](../../crates/replay/src/campaign/changes.rs)
