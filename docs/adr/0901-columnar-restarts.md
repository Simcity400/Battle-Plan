# 0901. Restart a columnar walk from a self-contained table state

A restore holds a table's state at the end of a block so a walk can start in the following block. It encodes status and acquisition metadata, then strings cited by live text cells or the refusal, then the table. Acquisition records the sample hour of the held view, the start of the table's availability, and any dictionary-backed refusal. The encoding records string ids in ascending order as gaps. Each string is length-prefixed.

The table records its slot count, live bitmap, and live keys in slot order. It gives every column's byte length before the values, allowing a projected reader to skip unwanted columns and leave their cells unrecorded. Each column also stores its unrecorded live slots. A truncated view names the read slots and cannot name a nonlive row. Every other status has no truncated-view list.

Some tables retain the last complete read alongside a partial current view. A complete view sampled at its read can stand in for that read directly. Store the hour of the last complete read. Add the older rows if they differ from the current ones. Before a partial read changes complete rows, copy their page handles so only pages written by the partial read need fresh allocations. When a complete read containing no section leaves the view unchanged, retain its prior complete read hour. Weak restart caches borrow decoded tables while another walk still holds every needed page.

[Restore codec](../../crates/replay-columnar/src/restart.rs) · [Paged storage](../../crates/replay-columnar/src/state/paged.rs)
