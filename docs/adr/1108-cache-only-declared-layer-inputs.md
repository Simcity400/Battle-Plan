# Cache only declared layer inputs

A decoded hour shares a table allocation with the previous hour when the table is unchanged. Use those allocations as cache keys for layers depending on a fixed set of tables. Keep the allocations alive while the key exists. Equal rows in a new allocation can cause a harmless cache miss. A layer that also reads metadata includes it in its own extra key because projection preserves the branch and hour independently of the table identities.

Build from a projected frame that exposes only declared table inputs. An undeclared read stays unavailable instead of creating a cache entry that may survive a later table change. When discovering an undeclared input on a held frame, leave its result out of the cache so the next request can rebuild with that input.

Build outside the shared lock. Callers needing the same key normally share one build. The path that cannot wait can build an independent copy without publishing over the first. Errors and unwinds settle the entry and let future callers retry rather than stranding waiters. Take the entry-list lock before the per-cell lock when both are required. Keep the latest value and other values only while callers hold them.

Some row fields change every hour even though the layer ignores them. For those layers, settle a new build on an equal existing value so downstream identity caches still hit. Two equal builds for distinct keys can complete concurrently before either can see the other's value. The extra identities cause at most a miss for the downstream use.

[Layer caches](../../crates/viewer-runtime/src/memo.rs) · [Cache contracts in tests](../../crates/viewer-runtime/src/memo/tests.rs)
