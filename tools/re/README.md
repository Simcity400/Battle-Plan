# hoi4re

One tool for reading the decompiled game and saving what you learn. Python 3.11, standard library only.
Run it from any shell as `python tools/re/hoi4re.py <command>` or `hoi4re <command>`
(`--help` lists them). `C:/Tools/hoi4_mcp/hoi4re` is a junction to this directory, so both run the same code.

## Data

Everything lives in `tools/re/data/` of the main checkout (``). It is git-ignored: it holds decompiled
game code and multi-GB databases. The tool finds it from a worktree too. Only `knowledge.sqlite3` and its journal hold
work that cannot be rebuilt: run `backup` (copies them to `L:/hoi4re_backups/<date>`) after any large batch of work.

| Path | What |
|---|---|
| `knowledge.sqlite3`, `knowledge.journal.jsonl` | What agents and scripts learned: `symbols` (names), `notes`, `fields`, `classes` and `vslots` (RTTI), `mentions`, `sources`, `accessors`, `queue`. Every agent write is also appended to the journal. |
| `function_index.sqlite3` | Decompile, call graph and full-text search for hoi4.exe 1.19.2.0.a729 (read-only). |
| `builds/a729/` | `index.sqlite3`: functions the first index missed, plus every function's byte and shape fingerprint for patch-day matching. `instructions.sqlite3`: compressed instruction listings for `asm`. New builds go in `builds/<id>/`. |
| `ghidra/a729/` | The Ghidra project of the a729 exe, for re-exports. |
| `fleet/` | Batch files and the orchestrator log of the naming fleet. |
| `backups/` | Local safety copies from maintenance. |

Known names replace `FUN_`/`DAT_` labels everywhere they are shown, as `Name@address`. `<local-path>` is a
junction to `fleet/`.

## Read

| Command | Use |
|---|---|
| `fn <addr or name>` | Name, summary, class, notes, mentions, callers, callees, matching fields, decompile. `--no-body`, `--max-lines N`, `--raw` |
| `search <fts query>` | Full-text search of decompiled bodies: `"instant_build"`, `NEAR(supply homebase)` |
| `xrefs <addr>` | Every function that references a global, vtable, string or function |
| `find <text>` | Names, classes and fields containing the text |
| `callers` / `callees <addr> [--depth N]`, `path <a> <b>` | Call graph (direct calls only; virtual calls are not edges) |
| `class <Name>`, `vtable <Name>` | Bases, vtables, fields, named methods; slot list |
| `field-users <Class> <offset>` | Functions touching an offset, those tied to the class first |
| `strings <literal>`, `data <addr>`, `asm <addr>` | Literal and its users; initialised bytes; instruction listing |


## Write (do this before you finish)

```
hoi4re name 1422198a0 CApplication::Run --summary "main loop; calls Frame until quit" --confidence high --evidence "SetThreadAffinityMask then loop on Frame"
hoi4re note 1401dcef0 "sets tick_busy (+0x4c0) before advancing the hour"
hoi4re field CGameState 0x4c0 TICK_BUSY --type u8 --meaning "set for the whole hourly tick"
hoi4re batch @findings.jsonl        # lines: {"op":"name","target":"...","name":"..."} | {"op":"note",...} | {"op":"field","cls":...}
```

Confidence is `low`, `medium`, `high` or `verified`; a name never replaces one of higher confidence (it is kept as a note).
Struct layouts used by the recorder stay authoritative in `crates/engine/src`; `seed engine` re-imports them.

## Maintain

- `seed [rtti] [engine] [docs] [sources]`: rebuild the imported layers (RTTI from the exe, the engine crate, reports
  and docs, source files named in assert strings). Re-runnable; agent-written rows are untouched. Run `seed engine`
  after engine changes. Names and notes imported once from the retired `hoi4_re` server's registry stay as they are.
- `derive [defines] [plumbing] [accessors] [ctors] [virtuals]`: names a script can prove alone, at confidence `derived`:
  define globals (with the value and comment from `00_defines.lua`), compiler plumbing (unwind funclets, adjustor thunks,
  static destructors), one-line accessors, constructors and destructors, and overrides of any named virtual. Naming a
  virtual re-runs `virtuals` automatically. Re-run the rest after `seed`.
- `export`: agent-written knowledge as JSONL (`batch` reads it back). `backup [--dest D]`: dated copy of the knowledge
  database and journal on another drive (`HOI4RE_BACKUP` changes the default).
- Patch day: `migrate export`, `migrate index`, `migrate names` (see `migrate.py`), then `HOI4RE_BUILD=<id>`.
  Matching order: RTTI slot, identical bytes, unique instruction shape, call-graph neighbours.

`HOI4RE_DATA`, `HOI4RE_REPO`, `HOI4RE_EXE` and `HOI4RE_BUILD` override the defaults.

## Fleet work

`todo fill` builds ranked queues; `todo next <kind> --author A --count 5` claims items and prints the brief to follow
(`briefs/RULES.md` plus one file per kind: `hot`, `cited`, `sourced`, `class`, `docs`); `todo status` shows what is left.
Function items close when the function gets a name at `low` or better; `todo done` closes the rest. Claims lapse after 3 hours.
Every agent write needs an author, and confidence above `medium` needs `HOI4RE_TRUSTED=1`.
`review authors`, `review sample --author A`, `review drop --author A` check and, if needed, remove one agent's work.
