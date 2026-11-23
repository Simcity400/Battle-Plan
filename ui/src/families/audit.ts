import type { AuditRow } from '../draw/notifications';
import type { Value } from './value.ts';

export interface FamilyAuditField {
  readonly id: number;
  readonly rows: readonly AuditRow[];
}

export function canonicalAuditValue(value: Value): string {
  switch (value.kind) {
    case 'zero':
      return JSON.stringify(['zero', '0']);
    case 'known':
      return JSON.stringify(['known', typeof value.value === 'bigint' ? value.value.toString() : Object.is(value.value, -0) ? '0' : String(value.value)]);
    case 'partial':
      return JSON.stringify([
        'partial',
        typeof value.value === 'bigint' ? value.value.toString() : Object.is(value.value, -0) ? '0' : String(value.value),
        value.reason,
        value.atLeast === true,
      ]);
    case 'notRecorded':
      return JSON.stringify(['notRecorded', value.reason]);
    case 'notApplicable':
      return JSON.stringify(['notApplicable', value.reason]);
  }
}

export function auditIdOf(field: unknown): number | undefined {
  if (!field || typeof field !== 'object') return undefined;
  const id = (field as { id?: unknown }).id;
  return typeof id === 'number' && Number.isInteger(id) && id > 0 ? id : undefined;
}

export function auditRowsOf(field: unknown): readonly AuditRow[] | undefined {
  if (!field || typeof field !== 'object') return undefined;
  const rows = (field as { rows?: unknown }).rows;
  return Array.isArray(rows) ? (rows as readonly AuditRow[]) : undefined;
}

export class AuditSnapshotStore<TSnapshot> {
  private readonly entries = new Map<number, TSnapshot>();
  private readonly maxEntries: number;
  private nextId = 1;

  constructor(maxEntries = 512) {
    this.maxEntries = Math.max(1, maxEntries);
  }

  create(snapshot: TSnapshot): number {
    const id = this.nextId++;
    this.entries.set(id, snapshot);
    if (this.entries.size > this.maxEntries) {
      this.entries.delete(this.entries.keys().next().value!);
    }
    return id;
  }

  get(id: number | undefined): TSnapshot | undefined {
    if (id === undefined || id <= 0) return undefined;
    return this.entries.get(id);
  }

  require(id: number | undefined): TSnapshot {
    if (id === undefined || id <= 0) {
      throw new Error('Not auditable: picture carries no snapshot id');
    }
    const found = this.entries.get(id);
    if (!found) {
      throw new Error(`Not auditable: snapshot ${id} is missing or evicted`);
    }
    return found;
  }

  update(id: number, patch: (current: TSnapshot) => TSnapshot): void {
    const current = this.entries.get(id);
    if (current !== undefined) this.entries.set(id, patch(current));
  }

  clear(): void {
    this.entries.clear();
  }

  get size(): number {
    return this.entries.size;
  }
}
