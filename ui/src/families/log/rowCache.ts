import type { LogCell, LogPageRow } from '../../generated/dashboard';
import type { LogLooksMap } from './declaration.ts';
import type { LogRowSpec } from './model.ts';

interface CachedRow {
  readonly signature: string;
  readonly spec: LogRowSpec;
}

const MAX_CACHE_ENTRIES = 512;

function cellFingerprint(cell: LogCell | undefined): string {
  if (!cell) return '';
  switch (cell.kind) {
    case 'text': return `t:${cell.value}`;
    case 'number': return `n:${cell.value}`;
    case 'list': return `l:${cell.bytes}`;
    case 'missing': return 'm';
    case 'notRecorded': return 'u';
  }
}

function rowFingerprint(columnsKey: string, wire: LogPageRow): string {
  let out = `${columnsKey}\x1f${wire.hour}\x1f${wire.ordinal}\x1f${wire.country ?? ''}\x1f${wire.ownOrdinal ?? ''}`;
  for (let i = 0; i < wire.cells.length; i++) {
    out += `\x1e${cellFingerprint(wire.cells[i])}`;
  }
  return out;
}

export class LogRowProjectionCache {
  private readonly byLooks = new WeakMap<LogLooksMap, Map<string, CachedRow>>();

  getOrProject(looks: LogLooksMap, columnsKey: string, wire: LogPageRow, build: () => LogRowSpec): LogRowSpec {
    let bucket = this.byLooks.get(looks);
    if (!bucket) {
      bucket = new Map();
      this.byLooks.set(looks, bucket);
    }
    const signature = rowFingerprint(columnsKey, wire);
    const hit = bucket.get(wire.key);
    if (hit && hit.signature === signature) {
      return hit.spec;
    }
    const spec = build();
    if (bucket.size >= MAX_CACHE_ENTRIES) {
      const oldest = bucket.keys().next().value;
      if (oldest !== undefined) bucket.delete(oldest);
    }
    bucket.set(wire.key, { signature, spec });
    return spec;
  }
}
