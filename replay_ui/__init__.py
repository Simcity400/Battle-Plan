"""replay_ui — the offline 5-screen REPLAY UI for Battle Plan.

Reads the canonical snapshot store (replay/snapshots/<game_id>/) READ-ONLY and
presents the five replay screens (map time-lapse, country detail, military /
divisions, combat log, event timeline) as a self-contained static web app served
over loopback. See replay_ui/README.md.

Modules:
  serve.py   — stdlib loopback server + launcher (rooted at the repo so the
               embedded map scrubber's asset paths resolve).
  series.py  — compact per-game SERIES builder (charts never fetch 5 MB snapshots).
  demo.py    — synthetic 3-snapshot demo store under replay_ui/demo_snapshots/.
  app.html / app.css / app.js — the front-end.
"""
__all__ = []
