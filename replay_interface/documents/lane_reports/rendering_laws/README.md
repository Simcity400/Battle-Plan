# Rendering-laws lane reports

This directory is the disjoint output surface for the execution packets in
`../../RENDERING_LAWS_RE_EXECUTION_PLAN.md`.

Rules:

- one packet owns one report file;
- reports bind the exact authority manifest in
  `../../../re_evidence/rendering_laws/AUTHORITY.json`;
- packets do not edit shared canonical law/status documents;
- Ghidra access is read-only, no-analysis, and serialized through the registered broker;
- no adversarial review runs before the final integrated review wave.
