import { occupiedDetail, validateOccupiedStates } from '../../widgets/occupied-states/occupiedStatesData.ts';
import { filterNext, filterStretch, occupiedTable, type OccupiedTableContext } from '../../widgets/occupied-states/occupiedTable.ts';
import type { TableRowModel } from './surface';

export interface BuiltTable { rows: TableRowModel[]; empty: string | null }

export type TableBuild = { kind: 'occupied-states'; context: OccupiedTableContext };

export function buildStretch(build: TableBuild, hour: number): number {
  switch (build.kind) {
    case 'occupied-states': return filterStretch(build.context.filter, hour);
  }
}

export function buildNext(build: TableBuild, hour: number): number {
  switch (build.kind) {
    case 'occupied-states': return filterNext(build.context.filter, hour);
  }
}

const read = new WeakMap<object, unknown>();
function checked<T>(reply: unknown, check: (reply: unknown) => T): T {
  if (!reply || typeof reply !== 'object') return check(reply);
  if (!read.has(reply)) { try { read.set(reply, check(reply)); } catch (error) { read.set(reply, error instanceof Error ? error : new Error(String(error))); } }
  const held = read.get(reply);
  if (held instanceof Error) throw held;
  return held as T;
}

export function buildDetail(build: TableBuild, reply: unknown, key: string): unknown {
  try {
    switch (build.kind) {
      case 'occupied-states': return occupiedDetail(checked(reply, validateOccupiedStates), key);
    }
  } catch { return null; }
}

export function buildTable(build: TableBuild, reply: unknown, hour: number): BuiltTable | null {
  try {
    switch (build.kind) {
      case 'occupied-states': return occupiedTable(checked(reply, validateOccupiedStates), build.context, hour);
    }
  } catch { return null; }
}
