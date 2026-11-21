# 0204o. Refresh province areas under the province census

The province walk acquires both area pointers with the movement fields. After a complete walk, its single take of the territory marks controls area reuse. An unchanged, quiet take can carry the held areas and read no extra pointers. A controller writer marks its province. An owner-area rebuild invalidates the full epoch. A bounded marked pass rereads possibly re-pointed areas, their former areas and neighbouring areas. If marking or pointer comparison cannot bound the set, carry falls back to a full read. Any provisionally carried pointer needed by the wider comparison must then be read again. A failed reread withdraws the same outcome a full walk would have refused and clears both holds. A missing store or incomplete parent walk publishes no area labels from a previous capture.

[Province area observer](../../crates/collector-observers/src/area_changes.rs)
