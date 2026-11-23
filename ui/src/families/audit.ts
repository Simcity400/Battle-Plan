import { useSyncExternalStore } from 'react';
import { auditing, onAuditingChange } from '../dashboard/pulled.ts';
import type { AuditRow } from '../draw/notifications';
import type { Scalar, Value } from './value.ts';

export function isFamilyAuditing(): boolean {
  return auditing();
}

export function subscribeFamilyAuditing(listener: () => void): () => void {
  return onAuditingChange(() => listener());
}

export function useFamilyAuditing(): boolean {
  return useSyncExternalStore(subscribeFamilyAuditing, auditing, auditing);
}

function isFamilyValue(input: unknown): input is Value {
  if (!input || typeof input !== 'object') return false;
  const kind = (input as { kind?: unknown }).kind;
  return kind === 'zero' || kind === 'known' || kind === 'partial' || kind === 'notRecorded' || kind === 'notApplicable';
}

export interface FamilyAuditField {
  readonly id: number;
  readonly rows: readonly AuditRow[];
}

function canonicalScalar(raw: Scalar): readonly ['str' | 'int' | 'num', string] {
  if (typeof raw === 'string') return ['str', raw];
  if (typeof raw === 'bigint') return ['int', raw.toString()];
  if (Object.is(raw, -0)) return ['int', '0'];
  if (Number.isFinite(raw) && Math.trunc(raw) === raw) return ['int', BigInt(raw).toString()];
  return ['num', String(raw)];
}

function canonicalValueNode(value: Value): readonly unknown[] {
  switch (value.kind) {
    case 'zero':
      return ['zero', ['int', '0']];
    case 'known':
      return ['known', canonicalScalar(value.value)];
    case 'partial':
      return ['partial', canonicalScalar(value.value), value.reason, value.atLeast === true];
    case 'notRecorded':
      return ['notRecorded', value.reason];
    case 'notApplicable':
      return ['notApplicable', value.reason];
  }
}

export function canonicalAuditValue(value: Value): string {
  return JSON.stringify(canonicalValueNode(value));
}

function canonicalAuditNode(input: unknown): unknown {
  if (input === null || input === undefined) return null;
  if (typeof input === 'boolean') return input;
  if (typeof input === 'string' || typeof input === 'number' || typeof input === 'bigint') {
    return canonicalScalar(input);
  }
  if (Array.isArray(input)) return input.map(canonicalAuditNode);
  if (isFamilyValue(input)) return canonicalValueNode(input);
  if (input instanceof Map) {
    const entries = [...input.entries()].map(([k, v]) => {
      const ck = canonicalAuditNode(k);
      const cv = canonicalAuditNode(v);
      return { ck, cv, sortKey: JSON.stringify(ck), sortVal: JSON.stringify(cv) };
    });
    entries.sort((a, b) => (a.sortKey < b.sortKey ? -1 : a.sortKey > b.sortKey ? 1 : a.sortVal < b.sortVal ? -1 : a.sortVal > b.sortVal ? 1 : 0));
    return ['map', entries.map(entry => [entry.ck, entry.cv])];
  }
  if (input instanceof Set) {
    const items = [...input.values()].map(v => {
      const cv = canonicalAuditNode(v);
      return { cv, sortKey: JSON.stringify(cv) };
    });
    items.sort((a, b) => (a.sortKey < b.sortKey ? -1 : a.sortKey > b.sortKey ? 1 : 0));
    return ['set', items.map(item => item.cv)];
  }
  if (typeof input === 'object') {
    const proto = Object.getPrototypeOf(input) as unknown;
    if (proto !== Object.prototype && proto !== null) {
      const name = (input as { constructor?: { name?: string } }).constructor?.name ?? 'Object';
      throw new TypeError(`Unsupported audit value type: ${name}`);
    }
    const keys = Object.keys(input as Record<string, unknown>).sort();
    const out: Record<string, unknown> = {};
    for (let i = 0; i < keys.length; i++) {
      const k = keys[i]!;
      const v = (input as Record<string, unknown>)[k];
      if (v !== undefined) out[k] = canonicalAuditNode(v);
    }
    return out;
  }
  throw new TypeError(`Unsupported audit value type: ${typeof input}`);
}

export function canonicalAuditData(input: unknown): string {
  return JSON.stringify(canonicalAuditNode(input));
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
