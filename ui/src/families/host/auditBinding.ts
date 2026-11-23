import type { ExactPicture } from '../../shell/perf.ts';
import type { AuditSnapshotStore } from '../audit.ts';

export type FamilyAuditVerifier<TSnapshot> = (
  hour: number,
  snapshot: TSnapshot,
  auditId: number | undefined,
) => ReturnType<ExactPicture>;

export interface BoundAuditEntry<TSnapshot> {
  readonly snapshot: TSnapshot;
  readonly verify: FamilyAuditVerifier<TSnapshot>;
}

export function createBoundAuditReader<TSnapshot>(
  store: AuditSnapshotStore<BoundAuditEntry<TSnapshot>>,
): ExactPicture {
  return async (hour, auditId) => {
    const entry = store.require(auditId);
    return entry.verify(hour, entry.snapshot, auditId);
  };
}
