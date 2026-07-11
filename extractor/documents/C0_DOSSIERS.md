# C0 DOSSIERS — pass-2 source-to-bit attribution (2026-07-11, Codex dataflow audit; tid in threads log)

Input: c0_domain_census.py pass-1 (13 unexplained pairs) + all 623 SOURCE/PARTIAL set-sites
with context + the reader function index. Verdict quality: 4 pairs FIRMLY classified from
code alone; 9 need either specific reader bodies (round 2, exact line ranges below) or the
ranked telemetry. KEY CONVERGENCE: the audit's ranked telemetry ledger IS C1 stage A's
per-subsource evidence-unit spec (G4 amendment 4) — one implementation serves both.

ROUND-2 INPUTS NEEDED (queued Codex job): lib.rs bodies read_compact_naval_combat_live
(22937-23410, d49+d50), read_compact_diplomacy_intel_operatives_live (19913-20949, d53),
read_compact_capitulation_war_state_live (21402-21624, d60), d51 reader/status fold from
17523, TRANSITION_BASELINE_RESET set sites, the frame-to-domain flag OR path, d48 gate.ok
dataflow — plus any lane-crate status producers those bodies call.

FIRM CLASSIFICATIONS (fail-closed policy confirmed correct for all):
- d45 PARTIAL = per-frame-structural (decisions.targeted_timed_live.structural_failure,
  site 38628). Never ExpectedAbsent/MeasuredEmpty.
- d65 SOURCE = rows.is_empty() WITHOUT a completeness witness (site 37474) — stays failed
  until a witness exists; d65 PARTIAL = dropped_ambiguous>0 (site 37480), documented
  owner-attribution gap, per-frame-structural.
- d54 PARTIAL = five routes (27110/27154/27166/27174/27250); 27250's skipped-rows causes
  are structural (assignment/linkage + path-geometry omissions); dependency routes need
  the branch vector.
- d51 pair = control transition; best hypothesis for exclusivity: a narrow transition
  frame in which d51 was the only emitted/census-attributed domain (baseline-reset marker
  concurs). Deciding unit: d51_control_transition_sample.

FULL AUDIT (verbatim):

Self-refutation: `bit_set_sites.txt` covers only exact `|= SOURCE_UNAVAILABLE/PARTIAL_DATA` expressions in `collector/src/lib.rs`. A status can also be inherited from a dependency or frame, constructed using different syntax, or returned by another crate. Therefore, “no listed site” means the producer is absent from the supplied evidence—not that no producer exists. The apparent d51 exclusivity could likewise be a scheduling window or a census-attribution artifact.

### Source-to-bit attribution

| Domain / bit | Visible set site and trigger | Cause classification | One deciding live evidence unit if needed | Fail-closed / future disposition |
|---|---|---|---|---|
| **d45 PARTIAL** | **38628**, owned by `read_compact_frd_depth_from_country_reads` (38286–38975): `decisions.targeted_timed_live.structural_failure` is true. This is the only direct visible producer in the reader bracket. | **per-frame-structural** | Context is sufficient. Useful confirmation: `d45_targeted_timed_structural_failure_frames {count, first_hour, last_hour}`. | Fail-closed is correct. Future typed disposition: structural/incomplete extraction, never `ExpectedAbsent`; not `MeasuredEmpty`. |
| **d48 PARTIAL** | **23463**, owned by `read_compact_construction_buildings_state_damage_live` (23411–24669): `hard_failure_flags != 0 || !gate.ok`. `hard_failure_flags` is every failure bit already in `final_status_flags` except PARTIAL itself. The named skipped-row problems plausibly feed `gate.ok`, but that dataflow is not in the excerpt. | **cannot-determine-from-context** | For each partial frame: `d48_partial_cause {hard_failure_flags, gate_ok, skipped_construction_rows, skipped_construction_non_construction_line_rows}`, aggregated with count and first/last hour per distinct tuple. | Conservatively correct. If the gate represents decode loss, use typed incomplete/structural failure. If non-construction rows are proven irrelevant, they could become a typed ignored-row condition—but only with a completeness witness for all actual construction rows. |
| **d49 PARTIAL** | **No set site supplied.** Its reader is `read_compact_naval_combat_live` (22937–23410), which contains no listed PARTIAL setter. Need the full body of `collector/src/lib.rs:22937–23410` and the source file(s) implementing every called lifecycle-status producer. | **cannot-determine-from-context** | `d49_status_cause {cause_id, count, first_hour, last_hour}` recorded where the lifecycle result first acquires PARTIAL. | Fail-closed is the only justified policy until the producer is known. Do not classify as `ExpectedAbsent` or `MeasuredEmpty`. |
| **d50 PARTIAL** | **No set site supplied.** Same owner bracket as d49: `read_compact_naval_combat_live` (22937–23410). Need its full body and the called member-extraction/status implementation. | **cannot-determine-from-context** | `d50_status_cause {cause_id, count, first_hour, last_hour}` recorded where the members result first acquires PARTIAL. | Same as d49: conservative fail-closed; future disposition depends on the missing producer. |
| **d51 TRANSITION_BASELINE_RESET** | **Not covered by the supplied set-site grep**, which searched only SOURCE/PARTIAL. Need all assignments/usages of `COMPACT_STATUS_TRANSITION_BASELINE_RESET` and the d51 reader/status-fold path beginning at **17523**. | **control-transition** | `d51_control_transition_sample`: one row per affected hour containing transition epoch, reset reason, scheduled domain IDs, d51 record presence, frame flags, and d51 record flags. | Fail-closed/reset semantics are correct: deltas must not cross a baseline discontinuity. Future typed disposition should be `BaselineReset`/`ControlTransition`, not absence or emptiness. |
| **d51 CONTROL_INVALID** | **Exact setter absent.** The orchestrator establishes that it is set at frame level when `control.scheduled_domains_valid == false`; the frame setter and frame-to-domain OR are not included. Need those `collector/src/lib.rs` regions. | **control-transition** | Same `d51_control_transition_sample`, additionally carrying `scheduled_domains_valid` before and after the transition. | Fail-closed is correct for invalid control input. Missing/invalid control is never `ExpectedAbsent`; use a typed control-transition or invalid-input disposition. |
| **d53 SOURCE** | **No set site supplied.** `read_compact_diplomacy_intel_operatives_live` spans 19913–20949, with no listed SOURCE setter. Need that full body and any called lane implementation returning its status. | **cannot-determine-from-context** | `d53_source_cause {cause_id, input_roots, emitted_rows, completeness_witness, count, first_hour, last_hour}`. | Conservative fail-closed is correct. It cannot become `MeasuredEmpty` without a positive completeness witness, and missing input/timeout cannot become `ExpectedAbsent`. |
| **d53 PARTIAL** | **No set site supplied.** Same missing reader/lane evidence as d53 SOURCE. | **cannot-determine-from-context** | `d53_partial_cause {cause_id, observed_rows, skipped_rows, count, first_hour, last_hour}` at the first PARTIAL-producing layer. | Fail-closed pending attribution. Future type could be torn, transient, capped, or structural; the excerpt cannot choose. |
| **d54 PARTIAL** | Five visible routes in `read_compact_army_battleplans_depth_live` (27101–27332): **27110** when incoming `final_status_flags` is already failure-class; **27154** when the omitted preceding test detects tolerated PARTIAL on army-orders; **27166** for tolerated PARTIAL on OOB-command; **27174** when either dependency has a hard failure, also adding SOURCE; **27250** when `skipped_rows != 0`, visibly including assignment/linkage omissions and path-geometry omissions. | Overall **cannot-determine-from-context**. Site 27250’s visible causes are **per-frame-structural**; the dependency routes could originate from churn, torn reads, caps, or structural loss. | `d54_partial_branch {inherited_failure, partial_army_orders, partial_oob_command, hard_army_orders, hard_oob_command, assignment_linkage_omitted, path_geometry_omitted, other_skipped}`, with count and first/last hour for every nonzero field. | Fail-closed is correct for hard dependency failure and omitted geometry/linkage. Future dispositions should retain the originating dependency/cause; no route shown supports `ExpectedAbsent` or `MeasuredEmpty`. |
| **d60 SOURCE** | **No set site supplied.** Reader `read_compact_capitulation_war_state_live` spans 21402–21624 with no listed SOURCE setter. Need that full body and any called lane status implementation. | **cannot-determine-from-context** | `d60_source_cause {cause_id, war_roots, capitulation_roots, emitted_rows, completeness_witness, count, first_hour, last_hour}`. | Fail-closed pending attribution. A zero-row result is `MeasuredEmpty` only with the completeness witness; missing input is never `ExpectedAbsent`. |
| **d60 PARTIAL** | **No set site supplied.** Same missing producer as d60 SOURCE. | **cannot-determine-from-context** | `d60_partial_cause {cause_id, skipped_wars, skipped_capitulations, count, first_hour, last_hour}` at first bit introduction. | Conservative fail-closed. Typed disposition cannot be chosen from the excerpt. |
| **d65 SOURCE** | **37474**, owned by `read_compact_special_project_programs` (37345–37536): `rows.is_empty()`. | **cannot-determine-from-context**: empty may mean genuinely no programs, unavailable inputs, or complete owner-join failure. | `d65_empty_witness {candidate_programs_scanned, candidate_facilities_scanned, owner_join_complete, emitted_rows, count, first_hour, last_hour}`. | Current fail-closed is correct because no completeness witness is shown. It may become `MeasuredEmpty` only when scanning and owner attribution are proven complete; never infer emptiness merely from `rows.is_empty()`. |
| **d65 PARTIAL** | **37480**, same reader: `dropped_ambiguous > 0`. The comment identifies ambiguous facility owner attribution as a documented known gap and deliberately marks the owner-joined set incomplete. | **per-frame-structural** | Context is sufficient. Confirmation counter: `d65_dropped_ambiguous_frames {count, total_dropped, first_hour, last_hour}`. | Fail-closed is correct. Future typed disposition: incomplete/ambiguous attribution, not `ExpectedAbsent` or `MeasuredEmpty`. |

### Why frame-level CONTROL_INVALID appears only under d51

The best-supported hypothesis is a narrow control-transition frame in which d51 was the only emitted or census-attributed domain. If frame flags are ORed into every record actually emitted in that frame, domains not scheduled or not emitted cannot inherit the bit. The simultaneous d51 baseline-reset marker supports that interpretation.

It is not proven. Other possibilities are that the census joins frame flags only to d51, or that d51 alone copies frame flags into its record. The deciding evidence is the proposed `d51_control_transition_sample`: affected hour, `scheduled_domains_valid`, scheduled domain IDs, emitted domain IDs, raw frame flags, and each record’s pre/post-OR flags.

### Ranked telemetry additions

1. **Cross-domain status-cause incidence ledger**  
   Emit `{domain_id, bit, cause_id, stage, count, first_hour, last_hour}` at the first point each bit is introduced—not merely where it is later propagated. This can close every classification pair, including lane-returned statuses.

2. **Lane-result cause vectors for d49/d50, d53, and d60**  
   Attach cause IDs plus observed/skipped/root counts to each lane result. This directly closes six currently source-less pairs. Live telemetry can classify them, although the missing source files are still required to report exact code line numbers.

3. **Completeness-witness samples for empty SOURCE results**  
   Record root discovery, scan completion, owner/join completion, and emitted rows for d53, d60, and d65. This decides whether zero is unavailable input or legitimate `MeasuredEmpty`.

4. **d51 control-transition trace**  
   Capture schedule validity, transition epoch, scheduled/emitted domains, and frame-to-record flag propagation. One unit closes both d51 bits and explains their apparent exclusivity.

5. **d54 PARTIAL branch vector**  
   Separates inherited dependency failures from linkage/geometry omissions, preventing one ORed PARTIAL incidence from conflating several causes.

6. **d48 gate-reason vector**  
   Separate hard inherited failure, construction-row skip, non-construction-line skip, and any other gate clause. This determines whether the current gate is detecting real incompleteness or conservatively rejecting irrelevant rows.