---
name: delete-comment
description: Audit and remove existing Battle Plan source comments in a repository or path the user names. Use for /delete-comment, comment cleanup, or a comment-ratchet follow-up.
---

# Delete comments

Install the scanner with `python -m pip install -r tools/comment-lint-requirements.txt` if needed. Run `python tools/check-comments.py --list [PATH ...]` to locate comments and `--count` with the same paths to measure comment-only lines. Read each candidate with its surrounding code, callers and tests. Compare with Git history when a claim might be obsolete.

- **Tautology** repeats the adjacent code. Delete it.
- **Stale** describes behavior or tooling that no longer exists. Confirm the current path and delete it.
- **Excuse** defends an implementation shortcut that still needs fixing. Verify the defect and use the issue tracker when authorized to file. Group duplicates by underlying defect, link the issue, then delete the comment. Keep the implementation fix in the issue.
- **Decision Record** explains a current constraint, tradeoff or invariant that the code cannot convey. Add or update a focused ADR in `docs/adr/` with the decision, its reason and affected code, then delete the comment.

Review syntax that changes tool behavior before deleting it. Preserve required licence notices, generated markers and test quarantine annotations. Rust doctests, JSDoc types and Rust documentation emitted by `ts-rs` can carry executable meaning. Preserve consumed Rust documentation as equivalent `#[doc]` values. Check generated site-label parity when removing comments near `Rva` declarations, since `tools/sites.py` currently uses physical line distance to select names.

Work in reviewable batches. Keep any per-comment classification notes outside the worktree. Run `python tools/check_changed.py` once and report audited paths, categories, issues and ADRs, the comment-only lines removed, and pending scope. The lint compares against the base and accepts only surviving legacy comment text in a file. A deletion provides no allowance for a new comment.
