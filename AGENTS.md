# Agent guidance

- Read `PROJECT.md` before changing product behavior, formats, or architecture. Change either guidance file only when the owner requests or approves it.
- Complete the agreed scope; report genuine blockers. Ask the owner only for decisions you cannot resolve from existing requirements or evidence.
- Reuse existing work and validation after checking their applicability; investigate contradictions and gaps. Never weaken expectations to make a test pass.
- Delete replaced implementations and compatibility paths unless the owner requires compatibility; check consumers and remaining references from the repo root.
- Preserve others' work. Resolve conflicts by hunk; never stash or replace whole files with `--ours`/`--theirs`.
- Keep changes and comments focused. Evidence must identify its source and support reproducible checks; hashes alone do not prove correctness.
- Each implementation lane uses its own worktree and obtains one independent read-only review of requirements, the full diff, and raw validation evidence. Fix findings with the same reviewer; approval must cover the final revision.
- The explicitly assigned orchestrator owns integration: verify lane approvals, cross-lane contracts, conflict resolutions, and end-to-end behavior. Investigate risks or evidence gaps without routinely repeating implementation reviews.
- Run applicable validation for changed behavior; repeat only when changes, failures, or findings justify it. Landing requires a passing integration gate. Report what passed, on which revision, and any limitations.
- Use per-worktree `.cargo-target` directories and shared sccache; never delete `C:\sccache_cache` or share target directories.
- Keep scratch on `C:` outside the repo; never commit logs, dumps, or build output. Clean up only resources you own; verify ownership before terminating a process.
- Read the relevant section of [agent operations](extractor/native/scripts/AGENT_OPERATIONS.md) when validating, landing, capturing, or using reverse-engineering evidence.
