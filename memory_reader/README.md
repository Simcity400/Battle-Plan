# Battle Plan — host-side memory reader (Phase 0)

Read-only scaffolding for reading live game state from the host's own running
`hoi4.exe`, on the host machine only. Background, rationale, risks, and the full
plan are in [`../RESEARCH_memory_reader.md`](../RESEARCH_memory_reader.md).

**This is observation only.** It opens the process with `PROCESS_VM_READ` (no
write rights) and never modifies the game. It runs on the host alongside the
game; reading memory is passive, so it cannot desync MP or affect other players.

## What's here

| File | Purpose |
|---|---|
| `winmem.py` | stdlib-`ctypes` library: attach read-only, enumerate the module, typed reads, pointer chains, AOB (anchor + `bytes.find`) scanning, exe SHA-256 fingerprint. |
| `phase0.py` | CLI that *verifies* discoveries read-only from Python — the harness we grow into the real reader. |
| `layout.example.json` | template for the per-build layout (addresses/offsets) the Cheat Engine discovery fills in. |

## Requirements

- **64-bit Python** (the game is 64-bit). Check: `python -c "import struct;print(struct.calcsize('P')*8)"` → `64`.
- Run as the **same Windows user** as the game (so `OpenProcess` is permitted). Admin not normally required for read.
- No third-party packages.

## The Phase-0 loop (you in Cheat Engine ⟷ Python verifies)

Cheat Engine does the *discovery* (it can scan, next-scan-on-change, find what
accesses an address, dissect structures — Python can't discover, only read).
Each finding gets verified read-only here and recorded in a `layout.json`.

First, prove the plumbing **without the game**:

```
python phase0.py selftest
```

Then, with HOI4 running:

1. **Attach + fingerprint** — `python phase0.py info`
   Records the exe SHA-256 (build gate) and module base/size.
2. **Date / daily tick** — find a counter in CE that ticks once per in-game day,
   then `python phase0.py watch 0x<addr> --type u32`. Confirm it advances on the
   day change and is stable when paused. Prefer a monotonic `num_days` counter.
3. **Player country + a factory count** — crib the `pPlayer` chain from the
   current FearLess 1.19.1 table in CE, then
   `python phase0.py chain 0x<pPlayer> 0x408 0x90 --read u32 --at 0x<mil_factory_off>`.
   Confirm it matches the UI (and changes when you change it in-game).
4. **Global country container (THE UNLOCK)** — in CE, search for the 8-byte
   player `Country*`, walk up to the array that holds it, then iterate it here
   and read the factory count for **two different** countries. Two correct,
   distinct countries = all-country iteration is proven.
5. **One division's province** — with `tdebug` province IDs on, scan a division's
   current province, move it, next-scan; verify here that the value tracks its
   *actual* location. This is the gate on the daily division time-lapse.

Fill each result into a copy of `layout.example.json`. Steps 1–3 are low risk;
**Step 4 is the make-or-break wall**; Step 5 opens the (harder) division wall.

## Handy commands

```
python phase0.py procs --filter hoi              # find the process name
python phase0.py read 0x7FF6xxxx0000 --type f32  # one typed read
python phase0.py read 0x... --type u32 --scale 1000   # show fixed-point value
python phase0.py chain 0x<base> 0x18 0xC --read u32    # resolve + read a chain
python phase0.py aob "48 8B 05 ?? ?? ?? ?? 48 89"     # find a code anchor
python phase0.py rip 0x<match> --disp 3 --len 7        # decode its global address
```

Addresses take hex (`0x..`) or decimal; `--rel` treats an address as
module-base-relative (useful when cribbing offsets from a CE table).

The global `--process NAME` (default `hoi4.exe`) goes **before** the subcommand,
e.g. `python phase0.py --process hoi4.exe info`.

## Non-negotiables (from the research)

- **Build-hash gate / fail closed.** Only run against an exe whose SHA-256 is in
  the layout manifest. An unknown build → refuse, don't guess.
- **Validators.** Reject implausible reads (bad tags, absurd counts, non-canonical
  pointers, non-monotonic date) rather than publish silent garbage.
- **Save oracle.** Once a month, melt the host autosave (rakaly) and diff a sample
  of memory reads against the labelled save; drift = layout broke → stop publishing.

These (hash gate + validators + save oracle) are the mandatory guard against
silent misreads after a patch — the single most dangerous failure mode.
