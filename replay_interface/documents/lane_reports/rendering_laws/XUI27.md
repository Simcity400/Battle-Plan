# XUI27 - terminal replay-layer freeze

Status: **PASS - REPLAY LAYER CERTIFIED; EXTRACTOR PHASE IS THE NEXT BOUNDARY**.

This is the clean demarcation requested before extractor completion. It
certifies only `replay_interface` / `portable_replay` production behavior and
committed replay fixtures. It does not certify extractor code, a new recording,
or extractor-to-replayer end-to-end playback.

## Mechanical closure and authorities

The final mechanical closure contains 294 current replay inputs: 260 Rust/Cargo
dependency sources, 29 committed runtime fixtures opened or enumerated by the
locked tests, and five workspace or compile-time inputs. It includes the current
portable schema/core, `selected_present.rs`, `production_renderer.rs`,
`ui_runtime.rs`, `app.rs`, `headless.rs`, `replay_fixture_gen`, and Cargo.lock.

Seven replay authorities are hash-bound and recursively current: the portable
schema, non-UI production contract, static-program consumer manifest, effective
VFS, playback profile, map-interpreter output, and M02/M03/A48/O04 replay
promotion. Current replay failures are zero.

## Locked verification

Two isolated `--locked` replay-only passes produced byte-identical receipts,
SHA-256 `dd60ceb232460fd7b97e8faedc51dbe9ad973d81a54ec2abf10904729b768457`:

- `replay_fixture_gen`: 2 passed, 0 failed.
- `game_ui`: 389 passed, 0 failed, 2 ignored.
- `replay_app`: 233 passed, 0 failed.
- Extractor packages built or tested: 0.

Both passes retained the same post-pass input digest,
`7c883ebcbe2e59788dbf895911333ac76e88c3d023927dd88831e38356d74333`.
The terminal manifest is
`PASS_REPLAY_LAYER_CERTIFIED_EXTERNAL_EXTRACTOR_BOUNDARY_PENDING`, SHA-256
`2af5b97e5424c8dd87df5431a91f188afa9cc8f2681a0c36c7f02dfc4b709c27`.
Independent recursive verification passed; receipt SHA-256
`ee7c5af361cbb85492724bcdb1e259e8e7b4a0d664aa024c9fc47c363428589c`.

## Non-certifying history and extractor boundary

The audit records 59 superseded replay-history failures and 28
extractor-boundary failures without promoting either. The older mixed UI
manifest contributes 22 replay-history rows (8 stale) and six extractor rows
(2 stale); every one has `certification_gate=false`. These counts do not weaken
the current 294-file replay certification.

The two isolated Cargo targets were cleaned after the freeze (19,658 files,
27.4 GiB total), and the XUI27 Python cache was removed.

## Frozen receipts

- `REPLAY_UI_FINALIZATION.json`: `aa08d15310c11273c51eadb678bdd07783512158cb89ca0fba68f75671c4c0b9`
- `replay_owned_freeze_preparation.json`: `19b04a3858e317d1d346400c40119eed78d140faafdee13e7e8f9fdce4973c03`
- `recursive_upstream_audit.json`: `09546c7bc826d377c31e50b14be8615fae0c5582fea3b096a9003e74869da2d3`
- `replay_owned_manifest.json`: `2af5b97e5424c8dd87df5431a91f188afa9cc8f2681a0c36c7f02dfc4b709c27`
- `replay_owned_terminal_verification.json`: `ee7c5af361cbb85492724bcdb1e259e8e7b4a0d664aa024c9fc47c363428589c`

No new Ghidra operation was required. Function address, job ID, artifact path,
and Ghidra manifest hash are `N/A`; this is a replay-owned mechanical freeze.
