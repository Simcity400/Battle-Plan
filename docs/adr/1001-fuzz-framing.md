# 1001. Preserve outer witnesses while testing malformed input and unwinds

Build the fuzz corpus from synthetic wire fixtures without a game installation. Use the first input byte to choose the single-hour or snapshot-and-delta fixture. Put the remaining bytes in the country table's snapshot or change frame. Rebuild the block page, run directory, and root with the recording writer so the reader can reach the frame even when its contents are malformed.

Native unwind tests use independently authored fixture code and handlers outside the game. The test client assigns each native export its declared ABI and does not install a Rust catcher. Its entry and argument remain valid throughout normal and foreign dispatch. Native handlers catch exceptions before returning to the non-unwinding Rust wrapper. The test records actual native continuation state, including nonvolatiles before the wrapper restores its caller. Unexpected native exceptions become an oracle failure inside the fixture DLL.

[Corpus builder](../../fuzz/seed_corpus.rs) · [Fuzz framing](../../fuzz/src/lib.rs) · [Native probe](../../tools/tap-foreign-unwind/probe.cpp) · [Probe wrapper](../../tools/tap-foreign-unwind/runtime.rs)
