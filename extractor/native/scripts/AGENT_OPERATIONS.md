# Agent operations

Procedural reference for agent work; `README.md` owns product requirements. Read only the relevant section.

## Validation and landing

- For code changes, run the touched crates' full relevant test modules and `bash extractor/native/scripts/lane_law_tests.sh`. An intended test selection matching zero tests fails. Documentation-only changes need diff and reference checks.
- Resolve independent review findings before landing; reuse validation that still covers the final revision.
- On `landing-tmp` (or the assigned integration branch), merge each approved tip with `git merge --no-ff`. Check success, `HEAD^2` against the lane tip, and content survival with `git diff <lane-tip> HEAD -- <lane-files>`; explain any integration differences.
- Run `reverify_engine_evidence.py`, then `powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\extractor\native\scripts\Invoke-Validation.ps1 -Mode Landing -AllValidators` from the repo root. The gate owns landing citation validation; do not run that verifier separately.
- Preserve results and exit codes. On failure, identify the responsible change against the base branch and fix it. On success, push the authorized destination (normally `landing-tmp:master`), verify lane tips are ancestors, and retire owned temporary resources.

## Engine evidence

- `hoi4_re`: call `re_status` first; update its registry through tools only. Limit the offline queue to four jobs.
- `hoi4_ghidra`: call `ghidra_status`, confirm the executable identity, and run `ghidra_verify` before relying on indexed evidence. Census and cross-analysis results are discovery, not proof.
- Store reproducible evidence and its generating script under `extractor/native/runs/<run>/`; use the `engine_evidence` crate pattern and `reverify_engine_evidence.py` for pinned artifacts.
- Promoted evidence claims cite artifact path, byte size, hash, executable build identity, and supporting dynamic or game-file evidence.
- Pin hashed files to LF with `.gitattributes`; source-scanning tests must tolerate CRLF (use `str::lines()`).

## Captures

- Lanes send bounded probe specifications to the orchestrator. Bounded live reads on a matching build have standing authorization; run one capture at a time, for 60 seconds.
- Build with `cargo build -p collector`; verify the loaded DLL is the resulting artifact and matches the intended source revision and game build. Do not infer identity from timestamps alone.
- Measurement captures use `-KeepSpoolAfterSeal -StreamCompression none`. Check records seen before trusting an audit.
- Allow `collector_record` to finalize; monitor `RECORDER_FINALIZE_PROGRESS_V1.json` and retain outputs. Stop the owned hooked game after completion.
- If the game clock stalls, inspect the actual game state. If it is on the end-game screen, relaunch using `-StopExistingGame` after verifying process ownership.
