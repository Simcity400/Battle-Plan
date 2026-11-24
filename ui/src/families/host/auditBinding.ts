import type { ExactPicture } from '../../shell/perf.ts';
import type { AuditSnapshotStore } from '../audit.ts';
import type { FamilyAuditRow } from '../auditRow.ts';
import type { BoundedReadView, FamilyProjection } from '../contract.ts';

export type FamilyAuditVerifier<TSnapshot> = (
  hour: number,
  snapshot: TSnapshot,
  auditId: number | undefined,
  fields?: Readonly<Record<string, unknown>>,
) => ReturnType<ExactPicture>;

export interface BoundAuditEntry<TSnapshot> {
  readonly snapshot: TSnapshot;
  readonly verify: FamilyAuditVerifier<TSnapshot>;
}

export function createBoundAuditReader<TSnapshot>(
  store: AuditSnapshotStore<BoundAuditEntry<TSnapshot>>,
): ExactPicture {
  return async (hour, auditId, fields) => {
    const entry = store.require(auditId);
    return entry.verify(hour, entry.snapshot, auditId, fields);
  };
}

export interface PlanAuditSnapshot<TModel, TOptions> {
  readonly projection: FamilyProjection<TModel, TOptions>;
  readonly options: TOptions;
  readonly exactReadView: (hour: number) => Promise<BoundedReadView | null>;
  readonly deriveRows: (model: TModel, hour: number, fields: Readonly<Record<string, unknown>> | undefined) => readonly FamilyAuditRow[];
  readonly ordered?: boolean;
}

export function createPlanAuditVerifier<TModel, TOptions>(): FamilyAuditVerifier<PlanAuditSnapshot<TModel, TOptions>> {
  return async (hour, snapshot, _auditId, fields) => {
    const exact = await snapshot.exactReadView(hour);
    if (!exact) return null;
    const model = snapshot.projection.project(exact, snapshot.options);
    const values = new Map<string, string>();
    for (const row of snapshot.deriveRows(model, exact.hour, fields)) {
      if (values.has(row.key)) throw new Error(`Audit key ${row.key} repeats`);
      values.set(row.key, row.value ?? '');
    }
    return { values, ordered: snapshot.ordered !== false };
  };
}
