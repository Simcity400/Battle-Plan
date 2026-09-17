# Job: cited (functions the repo and reports already describe)

Read `RULES.md` first. Each item is a function that source comments or reports mention but nobody named.

For each item from `hoi4re todo next cited`:
1. `hoi4re fn <addr> --max-lines 150`. The `mentioned` lines give file and line. Open those files at those lines
   (repo root ``) and read the surrounding paragraph.
2. Check the claim against the decompile. Comments can be outdated; trust the code when they disagree, and say so in a note.
3. Name it from what it does (`CCountry::AddCommandPower`, not `command_power_writer`). Put the file and line in `evidence`.
4. Add `field` lines for every class offset the comment and the code agree on.
