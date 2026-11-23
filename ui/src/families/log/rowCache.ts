import type { LogPageRow } from '../../generated/dashboard';
import type { LogLooksMap } from './declaration.ts';
import type { LogRowSpec } from './model.ts';

interface CachedRow {
  readonly ordinal: string;
  readonly hour: number;
  readonly cellCount: number;
  readonly firstVal: string;
  readonly spec: LogRowSpec;
}

const MAX_CACHE_ENTRIES = 512;

function firstCellToken(row: LogPageRow): string {
  const c = row.cells[0];
  if (!c) return '';
  if (c.kind === 'text' || c.kind === 'number') return `${c.kind}:${c.value}`;
  return c.kind;
}

export class LogRowProjectionCache {
  private readonly byLooks = new WeakMap<LogLooksMap, Map<string, CachedRow>>();

  getOrProject(looks: LogLooksMap, wire: LogPageRow, build: () => LogRowSpec): LogRowSpec {
    let bucket = this.byLooks.get(looks);
    if (!bucket) {
      bucket = new Map();
      this.byLooks.set(looks, bucket);
    }
    const firstVal = firstCellToken(wire);
    const hit = bucket.get(wire.key);
    if (hit && hit.ordinal === wire.ordinal && hit.hour === wire.hour && hit.cellCount === wire.cells.length && hit.firstVal === firstVal) {
      return hit.spec;
    }
    const spec = build();
    if (bucket.size >= MAX_CACHE_ENTRIES) {
      const oldest = bucket.keys().next().value;
      if (oldest !== undefined) bucket.delete(oldest);
    }
    bucket.set(wire.key, {
      ordinal: wire.ordinal,
      hour: wire.hour,
      cellCount: wire.cells.length,
      firstVal,
      spec,
    });
    return spec;
  }
}
