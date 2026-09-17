# Job: docs (mine one old report)

Read `RULES.md` first. The item is a file path. It was written during earlier reverse-engineering work and states what
certain addresses are. Your job is transcription with a check, not new analysis.

1. Read the whole file. List every statement that ties an address (`FUN_14xxxxxxx`, `0x14xxxxxxx`, `RVA 0x...`, where
   VA = 0x140000000 + RVA) to a meaning, and every `Class+0xNN` field statement.
2. For each address run `hoi4re fn <addr> --no-body`. Skip it if it already has a name at `low` or better that agrees.
3. Emit batch lines: a `name` only when the file states what the function is; otherwise a `note` quoting the finding.
   `field` lines for offsets. Confidence `low`, unless you opened the decompile and it clearly confirms: then `medium`.
   `evidence` is always `<file path>:<line>`.
4. If the file says a finding was refuted or superseded, save that as a note instead of a name.
5. Close the item: `hoi4re todo done docs "<path>" --note "N names, M notes, K fields"`.
