# Agent guidance

- Read [PLAN.md](PLAN.md) first, then [README.md](README.md). Work the current phase; do not start the next phase before its check passes.
- Work on one coordinated branch. Parallel helpers are owner-authorized; give them bounded tasks with clear file ownership. No lanes, landing branches or owner rulings in code comments.
- Design for aggregate cost as recorded data grows. Batch independent implementation and validation; shared captures must retain per-table correctness and cost results. Keep builds and heavy analysis outside timed game intervals.
- Keep one file per engine struct. The owner superseded the original under-100-file limit; report the final count without inventing a replacement cap.
- Replace, never shim. Delete replaced code and its references. A format change updates every reader and writer in the same commit.
- Validation is [PLAN.md section 5](PLAN.md#5-validation) and nothing else. Do not add test infrastructure, cloud CI, corpus tests or evidence reverification.
- Use the per-worktree `.cargo-target` and shared sccache at `C:\sccache_cache`. Never delete the shared cache.
- Captures belong under `captures\`, never in the repo. Keep scratch outside the repo. Never commit logs, dumps or build output.
- Only the owner changes decisions in `README.md`, `AGENTS.md` or `PLAN.md`. Apply owner-authorized amendments and record phase outcomes honestly.
