# 0301. Preserve known spans and rank boundaries across dashboard reads

Rank at each change boundary rather than rereading each hour. The sweep treats ending intervals before starting ones and keeps the endpoint after the maximum hour in a wider type. Share a boundary plan across measures while keeping their values and rank state separate. Ordinary top membership omits unknown values. Exact table ranking places active unknowns after known values and omits countries with no identity interval. Use exact comparison in both directions and break ties by recorded UTF-8 tag order. A callback that needs every supplying boundary uses rank positions, while coalesced rank runs merge unchanged neighbors and require independent bounds and stable keys. Shared clipping keeps the original supplying interval and lets the caller apply the recorded gaps and budget.

Keep retained runtime maps in their lookup form, and order hash-backed entries during serialization without copying their values. This gives serialized output a stable order while leaving the read path unchanged.

[Rank sweep](../../crates/dashboard-base/src/ranking.rs) · [Stable map serialization](../../crates/dashboard-core/src/map_pairs.rs)
