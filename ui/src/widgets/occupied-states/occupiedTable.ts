import { liveKey } from '../../charts/live.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import type { TableFormat } from '../../draw/tables/logic.ts';
import type { TableCellModel, TableRowModel } from '../../draw/tables/surface.ts';
import type { CountryLook } from '../../draw/tables/looks.ts';
import { emptyReason, garrisonCell, occupiedRows, STRENGTH, type CountryRole, type OccupiedStatesReply } from './occupiedStatesData.ts';

export const FORMAT: Record<string, TableFormat> = { resistance: 'fixed5Percent', compliance: 'fixed5Percent', garrison: 'fixed5Amount' };
const PERCENT_SCALE = 10_000_000;
const STATUS: Record<string, TableCellModel['status']> = { dot: 'dot', outline: 'outline', filled: 'fill' };

export type HourRanges = readonly (readonly [number, number])[];

export interface OccupiedFilter {
  passing: Readonly<Record<string, HourRanges>> | null;
  unrecorded: readonly (readonly [number, number, string])[];
  bounds: readonly number[];
}

export interface OccupiedTableContext {
  filter: OccupiedFilter;
  role: CountryRole;
  labels: Readonly<Record<string, readonly CountryLook[]>>;
  lawAs: string;
}

export interface OccupiedTable { rows: TableRowModel[]; empty: string | null }

const within = (ranges: HourRanges | undefined, hour: number) => !!ranges?.some(([first, last]) => first <= hour && hour <= last);

export function filterStretch(filter: OccupiedFilter, hour: number): number {
  let low = 0, high = filter.bounds.length;
  while (low < high) { const mid = (low + high) >> 1; if (filter.bounds[mid]! <= hour) low = mid + 1; else high = mid; }
  return low;
}

export const filterNext = (filter: OccupiedFilter, hour: number): number => filter.bounds[filterStretch(filter, hour)] ?? Infinity;

export function occupiedTable(reply: OccupiedStatesReply, context: OccupiedTableContext, hour: number): OccupiedTable {
  const { filter } = context, passing = filter.passing;
  const unrecorded = filter.unrecorded.find(([first, last]) => first <= hour && hour <= last)?.[2] ?? null;
  const view = occupiedRows(reply, passing ? tag => within(passing[tag], hour) : null, context.role, unrecorded);
  const empty = emptyReason(reply, view);
  if (reply.missing !== null) return { rows: [], empty };
  const laws = new Map(reply.laws.map(law => [law.token, law]));
  const redBelow = reply.garrisonRedRaw === null ? undefined : Number(reply.garrisonRedRaw);
  const country = (tag: string | null): TableCellModel => {
    if (tag === null) return { missing: 'Unknown country', iconSlot: true };
    const looks = context.labels[tag];
    return looks ? { looks, iconSlot: true, country: tag } : { missing: 'Country name unavailable', iconSlot: true, country: tag };
  };
  const rows = view.rows.map((row): TableRowModel => {
    const state = row.value;
    const value = (key: 'resistance' | 'compliance' | 'garrison', maximum?: number, missing?: string): TableCellModel =>
      ({ valueKey: liveKey(row.key, key), format: FORMAT[key], missing, maximum, country: state.occupier ?? undefined });
    const shape = garrisonCell(state);
    const law = state.law === null ? undefined : laws.get(state.law);
    return { key: row.key, cells: {
      state: state.name === null ? { missing: state.nameMissing ?? 'Not recorded' } : { text: state.name },
      owner: country(state.owner), occupier: country(state.occupier),
      resistance: value('resistance', PERCENT_SCALE),
      compliance: value('compliance', PERCENT_SCALE),
      garrison: { ...value('garrison', undefined, state.garrisonMissing ?? undefined), ...(shape.red && { tone: 'red' }),
        bar: shape.bar === null ? null : shape.bar === 'cell' ? { key: liveKey(row.key, STRENGTH), alertBelow: redBelow } : { fixed: 0 } },
      law: state.law === null ? { missing: state.lawMissing ?? 'Not recorded' }
        : { text: contentTitle(law?.name, state.law, 'Occupation law name unavailable'), status: STATUS[context.lawAs], icon: law?.icon ?? undefined, iconSlot: context.lawAs === 'icon' },
    } };
  });
  return { rows, empty };
}
