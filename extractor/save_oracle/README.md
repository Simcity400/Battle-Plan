# Save oracle

This directory turns a HOI4 save into offline JSON fixtures for development,
comparison, and parser testing. It accepts binary `.hoi4` saves and plaintext
`HOI4txt` saves; it is not part of live replay capture.

## Run a parser

Run commands from `extractor/save_oracle`:

```powershell
# Use the newest discovered save.
python parse_save.py

# Select a binary or plaintext save explicitly.
python parse_save.py "C:\path\campaign.hoi4"

# Select Rakaly and the output directory explicitly.
python parse_save.py --rakaly "C:\tools\rakaly.exe" --out-dir output "C:\path\campaign.hoi4"
```

`parse_save.py` writes `state_ownership.json`, `country_snapshot.json`, and
`meta.json`.
Other `parse_*.py` programs produce one bounded domain from the same input:

```powershell
python parse_units.py "C:\path\campaign.hoi4"
python parse_control.py "C:\path\campaign.hoi4"
python parse_focus.py "C:\path\campaign.hoi4"
python parse_production.py "C:\path\campaign.hoi4"
```

Use `python <parser>.py --help` for its accepted arguments. The exact mapping
from parser to JSON files is maintained in `output_contract.py`; do not infer
ownership from old output files. Outputs go to `output/` unless `--out-dir` is
given.

When no save path is supplied, discovery considers `*.hoi4` and
`*_melted.txt`, then chooses the newest file. Set `BATTLE_PLAN_SAVE_DIR` to add an
explicit search directory; the standard HOI4 save directory and Downloads are
also searched.

## Binary saves and Rakaly

Files beginning with `HOI4txt` are parsed directly. Files beginning with
`HOI4bin` require Rakaly. Supply it with `--rakaly`, or let discovery check
`PATH`, `.cargo/bin`, `AppData/Local/rakaly`, and Downloads.

The parser invokes Rakaly with the save path and captures `melt --to-stdout`
into a temporary plaintext file. Pass the save path directly when diagnosing
Rakaly; piping the save through standard input is not the supported invocation.

Rakaly can return exit code 1 for unknown mod tokens while still producing a
complete melt. The wrapper accepts that result only when the output starts with
`HOI4txt` and is larger than 1 KiB, and prints the sanitized warning. A missing,
empty, or non-plaintext result remains a hard failure. `-u stringify` preserves
unknown modded tokens instead of aborting their melt.

## Validate outputs

The cross-file validator expects every JSON file declared by
`output_contract.py` and rejects outputs from mixed saves:

```powershell
python validate.py
python validate.py --output-dir "C:\path\oracle-output"
```

`--allow-missing-outputs` and `--allow-stale-mix` are debugging escapes. Do not
use either to accept a fixture set.

Run the component tests from the repository root:

```powershell
python -m pytest extractor/save_oracle
```

## Troubleshooting

- `binary save ... rakaly was not found`: install Rakaly or pass its exact path
  with `--rakaly`.
- An unexpected save was selected: pass the save explicitly or set
  `BATTLE_PLAN_SAVE_DIR`; the default is newest-file selection, not filename order.
- Validator reports a stale mix: empty the chosen generated-output directory,
  rerun every required producer against the same save, and validate again.
- A field is absent or unresolved: inspect that output's `meta.notes` and the
  producing parser. Do not substitute values from a different save.
