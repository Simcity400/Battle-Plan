# XN16C — content-addressed capture canonicalization/import

Status: **PROVEN — import contract frozen; no current import because admitted captures do not exist**

**Current production disposition: PROVEN SYNCHRONIZED PACKAGE ADMITTED.** XN16Q binds the 775-row outer and 773-row inner manifests directly; the two-shot importer remains historical tooling. Refreeze receipt `10365b5de7628e1ab8a02593a0adff5b6ff3bedc65724cbe952bcec01fc170db`; verification `68409cbd92453c520929720d8ec6095154ac33af1fba4d1f42c8a5e5f1b67662` (88/88); manifest `aa7b15297e672d2fa651eb22e6d0ddbca6d226090d81e5b6a92e77c140b5b480`. Ghidra function/job/artifact/manifest: `null`.

XN16C defines the immutable import boundary after W2, R2 and the atomic XN16H
gate succeed. It does not run those validators, reinterpret their capture laws,
execute the renderer or merge the two captures. The normative contract is
`replay_interface/re_evidence/rendering_laws/XN16C/import_contract.json`.

## Exact import boundary

An import requires all of the following exact bytes:

1. a W2 result with schema
   `hoi4-rendering-laws-xn16w2-admission-result-v1` and status
   `ADMIT_EXACT_SYNCHRONIZED_TREES_MEASURE`;
2. an R2 result with schema
   `hoi4-rendering-laws-xn16r2-admission-result-v1` and status `ADMIT`;
3. an XN16H atomic receipt with schema
   `hoi4-rendering-laws-xn16h-combined-admission-v1` and status
   `ADMIT_XN16_TWO_CAPTURE_SET`, binding the SHA-256 of both supplied component
   results;
4. one exact ordered source manifest and source root per capture;
5. one ordered attempt ledger retaining every earlier rejection and ending with
   the two admitted result objects;
6. a destination content-addressed store.

XN16C pins every W2/R2/H report, contract, validator, verification and manifest
hash before reading capture inputs. It consumes XN16H's receipt rather than
re-running or bypassing the atomic execution gate.

## Preserved content

Source manifests remain exact byte objects. Their artifact arrays retain source
order; every normalized relative path is unique, local to its supplied source
root, and rechecked for exact size and SHA-256. The importer preserves:

- W2 pass identity order: terrain, water, rivers, trees, including each pass/CB
  identity from the admitted result;
- R2 identity order: terrain bundle, water bundle, river bundle, post-pass RTV,
  plus exact river CB0 `5866dccf4c7cb126`;
- manifest artifact order, lane order and attempt-ledger order;
- the exact admission-result, atomic-receipt, source-manifest, attempt-ledger,
  frozen dependency and frozen rejection-history bytes;
- all source artifact bytes and their sizes/SHA-256 values.

Objects use `objects/sha256/<first-two>/<remaining-62>`. Existing objects are
rehashes before reuse. New objects use a same-directory temporary file followed
by atomic replace. The import envelope uses NFC UTF-8 canonical JSON, sorted
keys, preserved array order, no whitespace or floats. Its SHA-256 is the import
ID and filename. The two captures remain separate ordered lanes inside that
envelope; XN16C performs no XN16M-style merge.

## Current fail-closed state

`current_no_import.json` reports `NO_IMPORT`, zero objects written, no import ID
and the exact nine missing-input codes for W2 result/manifest/root, R2
result/manifest/root, attempt ledger, XN16H atomic receipt and CAS root. This is
the required current result: neither admitted capture exists.

The verifier passes 30/30 with status
`PASS_CONTENT_ADDRESSED_IMPORT_CONTRACT_FROZEN_NO_CURRENT_IMPORT`. It also uses
an ephemeral tiny synthetic fixture solely to exercise canonical IDs, atomic CAS
writes, object paths, idempotence and atomic-receipt mismatch rejection. The
fixture is deleted on exit and is explicitly never capture or promotion
evidence.

## Operator interface

The exact command, source-manifest shape and ordered-attempt-ledger shape are in
`IMPORT_CARD.md`. Only process exit zero plus status
`IMPORTED_CONTENT_ADDRESSED_XN16_CAPTURES` is successful. Any other outcome is
`NO_IMPORT` and writes no import envelope.

## Frozen tuple and provenance

This is an offline import packet. Ghidra function address, job ID and artifact
path are **N/A**; no binary claim is made.

- Import contract SHA-256:
  `20057b89dbeacbce11a5a35a1d3fc4d6ee14d54f5f85c45b704bcd2439a4cab5`.
- Importer SHA-256:
  `b0ce9258b5f80855c9714aabb125a63cff4cc10ab09d3a4777ddb9385ec19416`.
- Current no-import result SHA-256:
  `45675981ed307cdc660e2116c5fb2d4c6ec865981be4993b921f310c695313a1`.
- Import card SHA-256:
  `d358c4150f000a28d339c3db47e9c82cc6d4f7a4dc1ed923628c574bf89bfcb8`.
- Verification SHA-256:
  `b890f4e2ef33251f5af6dc52a6dece76dadffb171925e391441511b4e2e5d916`.
- Evidence manifest SHA-256:
  `72a28033f0e389e21c32c194dfc0bf738efb11fa84cef1b14ccfcc8923488d68`.

XN16C changes no XN16 disposition, `PROGRAM_STATUS.md`, collector, renderer,
extractor, XN16H execution packet or XN16M merge logic.
