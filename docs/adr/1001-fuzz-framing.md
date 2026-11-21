# 1001. Preserve outer witnesses while testing malformed input and unwinds

Build the fuzz corpus from synthetic wire fixtures without a game installation. Give an inner block or frame mutation valid outer framing and recompute the trailer checksum so decoding reaches the changed data. Derive the index's hour span from complete payloads when possible. Keep the single-hour seed when malformed bytes prevent decoding, and preserve the trailer checksum layout when placing the last index entry. Multi-hour valid mutations can then exercise snapshot and delta baselines throughout a recording.

Native unwind tests use independently authored fixture code and handlers outside the game. The test client assigns each native export its declared ABI and does not install a Rust catcher. Its entry and argument remain valid throughout normal and foreign dispatch. Native handlers catch exceptions before returning to the non-unwinding Rust wrapper. The test records actual native continuation state, including nonvolatiles before the wrapper restores its caller. Unexpected native exceptions become an oracle failure inside the fixture DLL.

[Corpus builder](../../fuzz/seed_corpus.rs) · [Fuzz framing](../../fuzz/src/lib.rs) · [Native probe](../../tools/tap-foreign-unwind/probe.cpp) · [Probe wrapper](../../tools/tap-foreign-unwind/runtime.rs)
