# Preserve scripted-store identity and validation order

## Flag scopes

Country, state and global flag scopes use the same dense manager. A row's replay key combines the caller-supplied scope and owner with the native token. The reader copies the latest-set raw hour, signed value and expiry days. Positive remaining days decrement in the daily pass and remove the row at zero. The write witness conservatively answers that a write is possible when it cannot read a header or live row. Capacity beyond the live count is irrelevant to expiry.

Authenticate and decode the manager header and the token, date and value spans of each row, including both date interfaces. A scope reader may copy the manager header alongside its scope identity and pass the result through, including a failed header copy. Batch the same three spans the single-row reader uses. A gather can reach bytes beyond an invalid row, but the verdict stays in row order. The report preserves the first failure and reason and the exact omitted count of a truncated prefix. Sort successful output by token and reject duplicate identities in the prefix. These reads need neither allocation nor retry. The row-at-a-time fixture is an oracle for sparse faults and capacity boundaries.

## Variable scopes

Global, country and state scopes follow store pointers. Character stores are embedded and authenticated per roster occurrence. Persistent script writes complete synchronously. Daily politics and peace settlement complete before the hourly return, and a load joins its worker before it returns. Keep the recorded scope identity independent from the native seed word, which starts at one and is hashed as context changes. Seeds can be unavailable even when entries can be copied.

Authenticate a store's vtable and copy its storage header before consulting a carry plan. The header contains the storage pointer, declared entry count, bucket mask and probe tail. The probe tail describes the additional slots used to accommodate maximum probe distance. Copy the vtable, header and seed in that order so a later fault cannot obscure an earlier verdict. A carried scope still reads its identity, vtable, header and seed. A successful carry uses held entries and exact key bytes, with name starts relative to the supplied byte slice. A fresh read reports its outcome and, if complete, its entries in native slot order.

Allocate no arenas in the scope walk. Measure the full entry and key-byte demand, including omitted rows, in caller-owned storage. If a whole scope does not fit, report its demand without planning carry or emitting a partial scope. Retain authenticated empty scopes as complete rows. Physical hash order is independent of replay identity. Replay assembly sorts exact keys and checks for duplicates off the capture thread. Determine occupancy from the native probe-distance field, including when an unoccupied slot contains plausible stale bytes.

Read fixed blocks of nodes including empty slots. A heap name's terminator and the rest of its row can share one scattered read when there is room in the name arena. If the arena needs to grow, check the terminator first to preserve the original failure verdict.

[Flag manager](../../crates/engine-scripts/src/cflag_manager.rs) · [Flag row](../../crates/engine-scripts/src/cscript_flag.rs) · [Variable scopes](../../crates/engine-scripts/src/cvariables.rs)
