# replay-identity

The recorded content identity: the game build, the ordered DLC and mod layers
(order is part of the identity), the content coverage policy, the tool builds and
the launcher/memory roster verification, and each session's start
(`SessionStart`, `SampleBoundary`), with their validation, checked copies and
recorded sizes.

It depends only on [engine-allocation](../engine-allocation/README.md), not on the
recording format, so the content layer ([installed-content-core](../installed-content-core/README.md))
builds on it without the native readers or the replay chain, and a format change
does not rebuild the content layer. The wire form lives in
[replay-data](../replay-data/README.md)'s `identity` module (`IdentityCodec`), which
re-exports these types at `replay::identity`. Shared rules live in
[AGENTS.md](../../AGENTS.md).

`test-support` exposes `test_fixtures::identity`, a valid identity the wire-form
tests above encode.
