# Rules for every hoi4re job

Tool: `C:\Tools\hoi4_mcp\hoi4re.cmd` (below: `hoi4re`). Run `hoi4re --help` once. You only read decompiled code and
write names, notes and fields through the tool. Do not edit files, launch the game or run Ghidra.

1. Set your identity first: `set HOI4RE_AUTHOR=<your agent id>` (PowerShell: `$env:HOI4RE_AUTHOR="..."`). Use the same id for the whole session.
2. Get work with `hoi4re todo next <kind> --count 5`. Finish those before asking for more. Claims lapse after 3 hours.
3. Name what the code shows. When the behaviour is clear but the exact identity is not, give a descriptive name at `low`
   (`SsoString_Destroy`, `RefCounted_Release`) and explain in the summary. Only when you cannot tell what it does, save a
   `note` with what you saw and close the item. Do not invent a class or game meaning the code does not show.
4. Names: `Class::Method` in the game's style (`CCountry::AddStability`, `CString::Assign`, `PdxAlloc`). No spaces. Keep a class
   prefix only when the function really belongs to that class (it takes `this`, or sits in its vtable).
5. Confidence: `low` = inferred from structure or callers; `medium` = confirmed by a string, assert text, log message or a
   named caller/callee that leaves no doubt. `high` is not available to you.
6. Every name needs a one-sentence `summary` (what it does, under 25 words) and `evidence` (the literal string, the assert text,
   the offsets, or the callers that prove it).
7. Do not replace a name that already has confidence `low` or better unless your evidence is stronger; say why in the summary.
8. Save in batches: write JSON lines to a file and run `hoi4re batch @file`. Line formats:
   `{"op":"name","target":"1401dcef0","name":"CGameState::HourlyUpdate","summary":"...","confidence":"medium","evidence":"..."}`
   `{"op":"note","target":"1401dcef0","text":"...","evidence":"..."}`
   `{"op":"field","cls":"CGameState","offset":"0x4c0","name":"TICK_BUSY","type":"u8","meaning":"...","evidence":"..."}`
9. A function item closes by itself when you name it. If you cannot name it, close it with
   `hoi4re todo done <kind> <item> --note "why"` after saving a note with what you learned.
10. No pipes are needed: use `--max-lines N`, `--limit N`, `--no-body`. The tool works the same from PowerShell, cmd and bash.
11. Useful reads: `fn <addr> --max-lines 150`, `callers <addr>`, `callees <addr>`, `xrefs <addr>`, `class <Name>`,
    `vtable <Name>`, `search "<literal>"`, `find <text>`, `strings <literal>`, `data <addr>`.
