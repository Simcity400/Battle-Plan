> **Read `documents/DECISIONS.md` first.** It is the target authority and outranks this file.
> This file describes the system **as built**.
>
> Superseded here: the recorder is not private tooling for only the owner's games; the target serves any player, mod set, and DLC combination.

# Battle Plan / Replay

Private, read-only tooling for recording and replaying the owner's Hearts of
Iron IV multiplayer games.

**Three documents carry the whole current-state surface:**

- `documents/DECISIONS.md` — what the product is supposed to be. The authority;
  it outranks every specification in this repo.
- `documents/STATUS.md` — where each half stands, as built.
- `documents/SEAM_CONTRACTS.md` — what each half is waiting on from the other.

Read them in that order. Per-domain knowledge is condensed into
`extractor/documents/domains/`, one file per domain.

## Layout

| Path | Purpose |
| --- | --- |
| `extractor/` | Recording half: injected DLL collector, Rust recorder host, compact domains, save-oracle validation, static parsing. Deep ledgers in `extractor/documents/`. |
| `replay_interface/` | Playback half: replay app, map/UI rendering, container reader, RE evidence. Specs and the rendering-law docs in `replay_interface/documents/`; retained packet evidence under `replay_interface/re_evidence/`. |
| `portable_replay/` | Neutral format crates both halves statically link. |
| `documents/` | DECISIONS / STATUS / SEAM_CONTRACTS. |

Anything in an `archive/` folder is historical evidence, never the current
plan; the only one left is `extractor/documents/archive/`. The repo carries no
process/rules docs — that history stays in git.

## Common checks

```powershell
cargo metadata --manifest-path extractor\native\Cargo.toml --no-deps --format-version 1
cargo metadata --manifest-path replay_interface\native\Cargo.toml --no-deps --format-version 1
$env:PYTHONPATH = "extractor;replay_interface"
python -m pytest extractor\save_oracle extractor\game_defs extractor\coverage_audit
python extractor\coverage_audit\check_coverage.py
```

Production live capture:

```text
hoi4.exe -> one injected DLL collector/coordinator -> typed compact records -> Rust recorder -> .battleplan
```

Python is development/proof/validation tooling only, never the production hot
extraction loop.
