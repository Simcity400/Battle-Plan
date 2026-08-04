# Lane AL — queued Codex adversarial gate

Codex is usage-limited. ONE attempt was made with the standard shape and died on
the usage limit; per protocol it was NOT retried into.

- **thread_id**: `019fcd55-8e70-7390-a8a3-33fc67706a3f`
- **attempted**: 2026-08-04T15:12Z (host clock)
- **verbatim error**:
  `You've hit your usage limit. Visit https://chatgpt.com/codex/settings/usage to
  purchase more credits or try again at Aug 8th, 2026 1:32 PM.`
- **event stream**: `thread.started` -> `turn.started` -> `error` -> `turn.failed`.
  The reply file (`-o`) was created but is 0 bytes, so there is no review to read.
- **prompt**: `codex_review_prompt.txt` in this directory (complete and
  self-contained; carries the full project context, the engine evidence, the
  closed defect class and the explicit out-of-scope list).

## To resume after Aug 8 13:32

Resume BY THE CAPTURED THREAD ID — never `resume --last`:

```sh
D=.claude/worktrees/agent-a158c0655a38b552a/extractor/native/analysis/lane_al
OUT=$(mktemp /tmp/cdx_XXXXXX.txt); AJ=$(mktemp /tmp/cdx_XXXXXX.jsonl)
codex exec --json -s danger-full-access -c approval_policy=never --skip-git-repo-check \
  -m gpt-5.6-sol -c model_reasoning_effort=high -c service_tier=priority \
  -o "$OUT" resume 019fcd55-8e70-7390-a8a3-33fc67706a3f - < "$D/codex_review_prompt.txt" > "$AJ"
```

Note the thread was started but the turn never ran, so re-sending the prompt on
resume is correct — nothing was consumed.

## What the review still owes

The five review categories are in the prompt. The two I could least fully
self-verify, and which a reviewer should attack first:

1. **Reclassified slot guards.** Inside the engine's count, a null / non-heap /
   wrong-vtable slot now returns Partial/Invalid where it used to terminate the
   dereferences every element below the count with no null check, so the engine
   itself cannot tolerate a null there. That is an inference from one engine
   call site; a second opinion is worth having, because this is the one change
   that could turn a previously-clean capture partial.

2. **Under-read risk.** Whether the engine's count can ever be SMALLER than the
   live prefix the old heuristic walk would have collected. I believe not (the
   count is the engine's own extent and the old walk could only read a prefix
   plus stale tail), but it is an argument, not a measurement.

Both are settled for good by the next live capture, which now publishes
`d41_facility_failure_stage` / `_tag_db_id` / `_slot_index` /
`_count_sample`: a clean capture leaves all four at 0.
