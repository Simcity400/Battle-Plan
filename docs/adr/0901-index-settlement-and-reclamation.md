# 0901. Settle index relations after applying an hour's row changes

The column reader applies input mutations before the index engine. Keep the old rows for `Changed` and `Gone`, and reset rows before `Reread`. Restore includes keys held out of view. Signed and unsigned integer keys share an order-preserving encoding. Its length prefix continues past 127 groups, allowing arbitrarily wide keys without breaking the byte sort used by notifications.

Membership causes describe the net change after settlement. A member may have been touched by the time the reader has removed its old target, so retain the prior unresolved outcome in the index until settlement. The `(value, member)` pair identifies a net action. Copy borrowed notification handles inside the callback because settlement can reclaim them. Maintenance runs after settlement, and its cost is measured separately.

The storage pager retains single pages and extents across updates. The pools remain separate and both count toward the estimated gain from compaction. Fixed-width extent records fit in the admitted pager without overflow allocation. Removing a key can leave an overflow separator referenced until compaction repairs the tree. Recycling a freed leaf can immediately reuse it for the extent tree, so recycle overflow storage using the saved leaf contents.

[Index engine](../../crates/core-indexes/src/engine.rs) · [Integer key encoding](../../crates/core-indexes/src/key.rs) · [Pager](../../crates/core-indexes/src/store.rs)
