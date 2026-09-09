# Agent guidance

- Read the Rules section of [README.md](README.md) before changing product behavior, formats, or architecture.
- Only change `README.md` or `AGENTS.md` when the owner asks.
- When you replace code, delete the old path and every reference to it. Leave no compatibility shims.
- Build with a per-worktree `.cargo-target` directory and the shared sccache. Never delete `C:\sccache_cache`.
- Keep scratch files outside the repo. Never commit logs, dumps, or build output.
