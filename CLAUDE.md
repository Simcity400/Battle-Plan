# Battle Plan — agent operating rules

This is a **private** repo: a passive, offline, host-side multiplayer-stats / **full-game
replay** tool for the owner's own game group. Read `DESIGN.md` for the vision (maximal-
depth replay: economy, military, divisions, provinces, combat, focuses, decisions, diplomacy —
"everything you'd see in a save game"). It is the owner's own game, read-only/observational —
there is no anti-cheat concern.

## Codex is always on — NON-NEGOTIABLE

Read **`CODEX_PROTOCOL.md`** and follow it. Codex (the local CLI) is an adversarial second
reviewer that is **always on by default** for any substantive RE finding, offset hypothesis,
save-parse logic, or risky change. **Never ask the user whether to use Codex** — just use it
and report what it said.

**When you spawn ANY sub-agent, embed the Codex instruction in its prompt** (the exact Mode A
command from `CODEX_PROTOCOL.md` + "read CODEX_PROTOCOL.md, consult Codex adversarially before
reporting"). This is how the workflow survives across agents with no shared context.

## Hard safety rules (these caused a history rewrite once — do not repeat)

- **NEVER** commit the owner's personal home path (`C:\Users\Example\...` / OneDrive paths) or
  the gitignored `/tools/` dev-scripts folder. Scan staged content for the username / `OneDrive` /
  `C:\Users` before every commit.
- **No Claude attribution** in commits — omit the `Co-Authored-By` trailer.
- **No commits unless the user explicitly asks.**
- Keep large binary artifacts (map PNGs, raw/melted saves, reader dumps) out of git — gitignore
  them.

## Architecture (one line)

Two channels: **memory reader** (live, daily/hourly cadence, read-only `ReadProcessMemory` on
the host's `hoi4.exe`) for liveness, **save oracle** (rakaly-melted binary saves) for truth /
validation / deep layers. Offsets are **rediscovered per build** from save-derived invariants —
never hardcoded blindly. Fail closed: if memory rediscovery fails after a patch, fall back to
saves rather than emit guessed data.
