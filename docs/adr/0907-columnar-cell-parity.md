# 0907. Encode native and converted rows into the same columns

The capture adapter converts typed fields to the wire payloads used by row recordings. The length prefix belongs to the row wire, so a variable-length cell contains its payload. Row masks fit in an unsigned 64-bit word. Both column batches and per-row staging must produce equivalent sections and restart state.

A producer can assert a completed table is identical and skip visiting any rows. It can supply proven changes by key, known removals and prior row pairs. Equal copied operands, including text and lists, can then retain their accepted cells. A partial walk updates what it can prove without evicting the unvisited rows. Complete censuses can prove deletion; refused tables name the reason and keep accepted state. Staging an hour cannot mutate the accepted baseline until its manifest and section commit. With no changed or removed rows, a delta has no section. If staging fails partway through text lookup or row validation, retry must produce the same result as a writer that never failed.

[Native capture adapter](../../crates/replay-journal/src/native_columns/capture.rs) · [Parity tests](../../crates/replay-journal/src/native_columns/tests.rs)
