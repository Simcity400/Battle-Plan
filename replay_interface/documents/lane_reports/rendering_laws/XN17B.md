# XN17B — DLC combinations and fail-closed composition

Status: **PROMOTION_READY_FAIL_CLOSED — XN17B admission law proven; four upstream producer/compose dependencies remain rejected**

XN17B is a separate extractor/replay-layer packet. It does not replace or amend the earlier `XN17` final-composition report, and it does not review any upstream packet. Its normative evidence is `replay_interface/re_evidence/rendering_laws/XN17B/census.json`; `verify_xn17b.py` rebuilds the complete installed namespace and compares it byte-for-byte.

## Exact production law

Construct the effective namespace in this order:

1. base game, excluding the physical `dlc` and `integrated_dlc` roots;
2. every `integrated_dlc` directory, sorted case-insensitively by id;
3. the union of active descriptor-backed product DLC and descriptorless asset layers, sorted case-insensitively by id;
4. active mods in launcher order.

Archive-backed DLC is enumerated as normalized archive members, not as its binding directory. Within every namespace reject unsafe paths, symlinks, empty/invalid required archives, duplicate case-folded paths, and any file or namespace that changes while being hashed. Later namespaces win an exact case-folded relative-path collision. `replace_path` removes lower-layer rows only when their immediate parent directory exactly equals the normalized replacement path; it is nonrecursive. No installed DLC descriptor in this a729 installation contains `replace_path`, but the generic composition gate preserves that proven UI15 rule for modded playsets.

The replay header must pin the executable identity, ordered source-inventory digest, effective rows digest, every consumed static catalog, and each producer manifest. A consumer recomputes and validates those documents before decoding packets. Any absent, mismatched, unfrozen, ambiguous, or drifted input rejects the playset, producer, packet, or frame at the earliest applicable boundary. It may never select a visually similar fallback.

## Exhaustive installed result

The installed tree contains:

- 24 descriptor-backed optional product DLC, hence exactly `2^24 = 16,777,216` product-DLC subsets governed by the same law;
- three descriptorless asset layers: `dlc014_wallpaper`, `dlc023_man_the_guns`, and `dlc048_content_creator_pack_peace_for_our_time`;
- four always-present integrated layers: Together for Victory, Death or Dishonor, Waking the Tiger, and Man the Guns;
- one archive-backed product, `dlc003_rocket_launcher_unit_pack`, whose ZIP has 27 normalized members;
- 32 total namespaces and 58,296 source rows, containing 10,046,340,752 bytes.

The exhaustive candidate graph has 57,963 unique case-folded paths and 304 multi-layer overlap groups. Forty-six groups contain byte-identical candidates; 258 contain different bytes; the maximum is 28 candidates for one path. The complete candidates, including all non-winning rows, are retained in `census.json` rather than reduced to a winner-only inventory.

For the retained RenderDoc playset, the live and captured `dlc_load.json` files are byte-identical. It has zero mods, 20 active product DLC, and four disabled products: `dlc013_sabaton`, `dlc019_sabaton_vol2`, `dlc025_axis_armor_pack`, and `dlc044_expansion_pass_1_ride_of_the_valkyries`. Its exact effective namespace contains 57,307 rows and 9,778,002,568 bytes. This capture authenticates one exact playset; it is not falsely presented as observation of every subset. Other subsets are admitted only by exact inventory recomputation under this generic law.

## Controlled missing-data contract

An inactive optional DLC is valid absence: do not mount it and do not synthesize its producers. That is distinct from missing selected content.

The corrected XN12A refreeze includes the always-active `integrated_dlc/dlc023` namespace. It proves 280 particle-bearing files, 180/180 resolved registrations, 18,238 entity particle events, and **zero unresolved references**. The four amphibious-water rows previously classified as engine-null resolve through integrated DLC023; XN17B admits no null amphibious rows.

Selected descriptors/content, invalid archives, unsafe or ambiguous paths, case-fold collisions within a namespace, source drift, and inventory/catalog hash mismatches reject the playset before packet decode. A missing packet asset, material, shader, semantic scope, sequence range, palette, or frozen producer manifest rejects that packet or frame. Unknown script constructs reject their semantic scope; they are not coerced to false, zero, or a default. Explicitly measured empty output remains valid only when the producer contract defines and records it.

## Current upstream admission dependencies

XN17B itself is ready to promote as an admission gate. The following data cannot enter production composition yet:

- **XN13:** requires a frozen entity-animation/per-pass palette producer manifest.
- **XN14:** requires a frozen weather-particle producer manifest.
- **XN15:** its proven recorder/container contract still requires a mechanically frozen manifest.
- **XN16:** reject final promotion until synchronized water+rivers, rivers pass-close RTV, and PAN2 clear provenance are captured and its final compose check is frozen.

These are typed dependencies, not reviews. If a dependency document appears but its hash is not pinned by a new XN17B integration result, it remains rejected.

## Binary provenance

XN17B makes no new binary claim and therefore has no Ghidra tuple. Its composition law consumes the mechanically frozen UI15 resolver contract and corrected XN12A static-catalog manifest. Any future binary assertion added to this packet must include function address, job id, artifact path, artifact SHA-256, and manifest SHA-256.

## Mechanical closure

`verify_xn17b.py` passed a fresh exhaustive rebuild and canonical whole-document comparison, then independently checked 25 invariants, all input/anchor hashes, inventory and winner digests, dependency policy, and the absence of stale engine-null claims.

- census internal SHA-256: `b1f46cb4256234b2592c606c1a0c61f0bd14b789acc0acc9949c9e8c999275f3`
- source inventory SHA-256: `eafe57c5dab7944d531f8fd560ac3b82355ec93b952c0a7abd91432da3d71538`
- source rows SHA-256: `37259cbc7c49521cb27799a64d3d5be3830a6d7f85ebb4736e8245aa30c3df3f`
- retained effective rows SHA-256: `7ec0349afd0ebaaaaa136774bce07f022cbc9f8c1b0f3e0ef5657ad55cdace00`
- overlap graph SHA-256: `622a291988fbe699efbb45cf8dfa2d70b03c716e1f0011300140830ca236bd74`

No excluded rendering item was investigated. `PROGRAM_STATUS.md` was not edited.
