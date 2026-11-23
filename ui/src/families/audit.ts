import { useSyncExternalStore } from 'react';
import { auditingDesired, onAuditingChange } from '../dashboard/pulled.ts';
import type { AuditRow } from '../draw/notifications';
import type { Scalar, Value } from './value.ts';

export function isFamilyAuditing(): boolean {
  return auditingDesired();
}

export function subscribeFamilyAuditing(listener: (active: boolean) => void | Promise<void>): () => void {
  return onAuditingChange(listener);
}

export function useFamilyAuditing(): boolean {
  return useSyncExternalStore(
    listener => onAuditingChange(() => { listener(); }),
    auditingDesired,
    auditingDesired,
  );
}

function isPlainObject(input: object): boolean {
  const proto = Object.getPrototypeOf(input) as unknown;
  return proto === Object.prototype || proto === null;
}

function isValidScalar(value: unknown): value is Scalar {
  if (typeof value === 'bigint') return true;
  if (typeof value === 'number') return Number.isFinite(value);
  if (typeof value === 'string') return value.trim().length > 0;
  return false;
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0;
}

function isFamilyValue(input: unknown): input is Value {
  if (!input || typeof input !== 'object' || Array.isArray(input) || !isPlainObject(input)) return false;
  const obj = input as Record<string, unknown>;
  const keys = Object.keys(obj).filter(k => obj[k] !== undefined);
  switch (obj.kind) {
    case 'zero':
      return keys.length === 2 && (obj.value === 0 || obj.value === 0n);
    case 'known':
      return keys.length === 2 && isValidScalar(obj.value) && obj.value !== 0 && obj.value !== 0n;
    case 'notRecorded':
    case 'notApplicable':
      return keys.length === 2 && isNonEmptyString(obj.reason);
    case 'partial':
      if (!isValidScalar(obj.value) || !isNonEmptyString(obj.reason)) return false;
      if (keys.length === 3) return true;
      return keys.length === 4 && typeof obj.atLeast === 'boolean';
    default:
      return false;
  }
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
  private readonly pinned = new Map<number, number>();
  private readonly stale = new Set<number>();
  private readonly maxEntries: number;
  private nextId = 1;

  constructor(maxEntries = 512) {
    this.maxEntries = Math.max(1, maxEntries);
  }

  private evictUnpinned(): void {
    if (this.entries.size <= this.maxEntries) return;
    for (const key of this.entries.keys()) {
      if (this.entries.size <= this.maxEntries) break;
      if (this.pinned.has(key)) continue;
      this.entries.delete(key);
      this.stale.delete(key);
    }
  }

  create(snapshot: TSnapshot): number {
    const id = this.nextId++;
    this.entries.set(id, snapshot);
    this.evictUnpinned();
    return id;
  }

  pin(id: number): (() => void) | undefined {
    if (!this.entries.has(id)) return undefined;
    this.pinned.set(id, (this.pinned.get(id) ?? 0) + 1);
    let released = false;
    return () => {
      if (released) return;
      released = true;
      const count = this.pinned.get(id);
      if (count === undefined) return;
      if (count > 1) {
        this.pinned.set(id, count - 1);
        return;
      }
      this.pinned.delete(id);
      if (this.stale.delete(id)) {
        this.entries.delete(id);
      } else {
        this.evictUnpinned();
      }
    };
  }

  isPinned(id: number): boolean {
    return this.pinned.has(id);
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
    for (const id of [...this.entries.keys()]) {
      if (this.pinned.has(id)) this.stale.add(id);
      else this.entries.delete(id);
    }
  }

  get size(): number {
    return this.entries.size;
  }
}
