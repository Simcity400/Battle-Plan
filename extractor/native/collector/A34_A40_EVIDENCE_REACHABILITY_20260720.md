# A34/A40 evidence acquisition reachability

Status: **owner-runnable, production-unscheduled**.

The injected collector now owns explicit acquisition lifecycles for the two
reserved compact domains:

- `A34DecisionPanelEvidenceOwner` installs the exact sentinel-gated decision
  hook graph in evidence mode, arms one engine-update epoch, seals its opaque
  hook receipt, joins only non-identity row state, destructively drains one
  DPN1 payload, and disables/waits/removes the hooks.
- `A40SelectedDesignerEvidenceOwner` installs the existing d78 order
  prerequisite and the four exact selected-designer detours, destructively
  drains one SDI1 payload per observed family, and disables/waits/removes the
  A40 detours. The independently owned d78 observer is deliberately retained.
- `ReservedDomainEvidenceRecord::encode_h4cr` creates an ordinary domain-data
  H4CR record only when its authenticated payload hour matches the enclosing
  hour. Sequence allocation remains with the recorder/harness authority.

Neither owner is present in the compact scheduler. Domain 85 and domain 87
remain `Reserved`, `schedulable=false`; the A40 six-family fixture latch and
the A34 production/live-fixture latches remain false.

## Identity boundary

The evidence paths accept no portable selected-designer identity at all. A40
removes the selected stable id and exact controller order from lane-owned hook
state while draining. A34 callers can resolve category, lifecycle, target,
numeric cost, and timer state, but cannot provide decision id, presentation
index, exact displayed name, or alert membership. Those fields are copied only
from the one-shot materialization receipt, and any supplied exact cost/timer
text must match a setter witness before DPN1 encoding.

## Remaining external evidence

This closes autonomous harness reachability, not the live acceptance gates.
Promotion still requires running the existing A34 fixture and A40 six-family
opening/transition fixture against the pinned executable, retaining the H4CR
payloads and their install/drain/uninstall receipts, then reviewing those
observations before changing either compile-time activation latch.
