# 0901. Account for spool storage until its bytes are released

Disk charges survive closing a handle because the file still occupies disk space. Consumer scratch keeps a dense prefix of pages and uses associative paging for overflow. Oversized reasons stream into owned spill pages. A carried cell owns a reason mapping that remains valid when the next segment reuses its dictionary ids. Validate every byte of a dictionary entry before lending any portion of it so a failed entry cannot leave a consumer with a partial value.

LZ4 admission reserves the larger of the two compression hash tables as an upper bound. Its output buffer meets `get_maximum_output_size`, 20 bytes plus 110 percent of the input. The benchmark calibrates thread-cycle readings against the invariant timestamp counter. The multiplied journal permits sparse inserts past the end of a table for development measurements.

[Spool budget](../../crates/core-spool/src/budget.rs) · [Dictionary](../../crates/core-spool/src/dictionary.rs) · [Writer](../../crates/core-spool/src/writer.rs) · [Consumer benchmark](../../crates/core-spool/src/bench/consumer.rs)
