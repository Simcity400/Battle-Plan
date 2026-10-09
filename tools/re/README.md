# hoi4re

Read the decompiled game and save native names, fields and notes in a shared knowledge database. The CLI uses Python 3.11 and the standard library. Run `python tools/re/hoi4re.py <command>` from the checkout, or `C:/Tools/hoi4_mcp/hoi4re.cmd <command>` from any shell; `--help` lists commands. The installed `hoi4re` directory is a junction to this tool.

Recorder layouts remain authoritative in [engine](../../crates/engine/README.md). Follow [AGENTS.md](../../AGENTS.md) for native evidence and publication rules.

## Files and private data

| File | Responsibility |
|---|---|
| [hoi4re.py](hoi4re.py) | CLI, decompile lookup, call graph, knowledge writes and backups. |
| [rtti.py](rtti.py), [seed.py](seed.py), [derive.py](derive.py) | PE/RTTI reading, imported knowledge and mechanically derived names. |
| [migrate.py](migrate.py), [ghidra/ExportAll.java](ghidra/ExportAll.java) | New-build export, indexing and symbol matching. |
| [work.py](work.py), [briefs](briefs/) | Naming queues, claims and review commands. |

Data lives in ignored `tools/re/data/` in the main checkout, including when run from a worktree. `HOI4RE_DATA`, `HOI4RE_REPO`, `HOI4RE_EXE` and `HOI4RE_BUILD` override defaults.

| Data | Purpose |
|---|---|
| `knowledge.sqlite3`, `knowledge.journal.jsonl` | Shared names, notes, fields, RTTI and work queues; every agent write also enters the journal. |
| `function_index.sqlite3` | Read-only decompile, call graph and full-text index for the original indexed build. |
| `builds/<id>/` | Supplemental/new-build index, byte and instruction-shape fingerprints, and compressed instruction listings. |
| `ghidra/<id>/` | Private Ghidra projects for export. |

The knowledge database and journal contain work that cannot be rebuilt. Run `backup` after a large batch; the default destination is `L:/hoi4re_backups/<timestamp>`, overridable with `--dest` or `HOI4RE_BACKUP`. Keep decompiles, databases and exports outside Git.

## Read native code

| Command | Use |
|---|---|
| `fn <addr or name>` | Symbol, notes, callers, callees, fields and decompile; `--no-body`, `--max-lines N`, `--raw`. |
| `asm <addr>`, `data <addr>` | Indexed instructions and initialized bytes. |
| `search <fts query>`, `find <text>` | Search decompiled bodies or saved names, classes and fields. |
| `xrefs <addr>`, `strings <literal>` | Functions referencing an address or string. |
| `callers <addr>`, `callees <addr>`, `path <a> <b>` | Direct-call graph; virtual calls are not graph edges. |
| `class <Name>`, `vtable <Name>` | Bases, fields, named methods and virtual slots. |
| `field-users <Class> <offset>` | Functions touching an offset, class-associated functions first. |


## Save findings

Set `HOI4RE_AUTHOR`, pass `--author`, or put `author` in each batch row. Save established findings before finishing:

```powershell
$env:HOI4RE_AUTHOR = 'your-name'
hoi4re name 1422198a0 CApplication::Run --summary "main loop; calls Frame until quit" --confidence medium --evidence "SetThreadAffinityMask then loop on Frame"
hoi4re note 1401dcef0 "sets tick_busy (+0x4c0) before advancing the hour"
hoi4re field CGameState 0x4c0 TICK_BUSY --type u8 --meaning "set for the whole hourly tick"
hoi4re batch '@findings.jsonl'
```

Batch JSONL rows use `op: name|note|field` and the corresponding command's fields. Confidence is `low`, `medium`, `high` or `verified`; a lower-confidence replacement stays as a note. Agent writes above `medium` are capped unless `HOI4RE_TRUSTED=1`.

## Maintenance and queues

- `seed [rtti] [engine] [docs] [sources]` rebuilds imported layers without changing agent-written rows. Run `seed engine` after layout changes.
- `derive [defines] [plumbing] [accessors] [ctors] [virtuals]` assigns mechanically established names at `derived` confidence. Naming a virtual reruns `virtuals`; rerun the other derivations after seeding.
- `export` writes non-RTTI symbols, notes and fields as JSONL that `batch` can import.
- For a new executable, run `migrate export`, `migrate index` and `migrate names` as described in [migrate.py](migrate.py), then select `HOI4RE_BUILD` and `HOI4RE_EXE`. Matching uses RTTI slots, identical bytes, unique instruction shape and call-graph neighbours. Export requires `HOI4RE_GHIDRA` pointing to a Ghidra installation with `support/analyzeHeadless.bat`; Java settings come from the environment. Indexing and name matching do not require Ghidra.
- `todo fill`, `todo next <kind> --author A --count 5` and `todo status` populate, claim and inspect naming work. Follow [briefs/RULES.md](briefs/RULES.md) and the claimed kind's brief. Function items close at confidence `low` or better; `todo done` closes other items, and claims expire after three hours.
- `review authors`, `review sample --author A` and `review drop --author A` inspect or remove an author's work.
