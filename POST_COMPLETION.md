# Post-completion list

Not to be staffed before perfect capture, certification, and the end-to-end
self-test are done. Owner-approved 2026-08-15.

1. **One shared format library.** One crate defines the replay-file format;
   recorder writes and replay reads through it. Producer/consumer drift becomes
   a compile error instead of a gate finding. Supersedes the drift-gate scripts.
2. **Instant open via seal-block indexing.** The viewer indexes sealed hour
   blocks progressively so long recordings open in seconds. Hard rule: it may
   only ever read SEALED blocks — the file stays the only bridge between the
   two apps, and an in-progress recording is never shared state.
3. **Round-trip golden gate.** Encode → decode → byte-identical over a golden
   corpus, run on every change. Wire the existing fold-hash parity and fixtures
   into one standing gate.
4. **One-app shell.** Ship recorder and viewer as one app: recording library,
   "ready — watch now" on seal, versions always matched. Packaging only — the
   internal file seam is unchanged.
