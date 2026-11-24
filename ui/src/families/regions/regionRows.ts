import type { ActiveRegions, NoRegions } from '../../generated/dashboard';
import type { BoundedReadView } from '../contract.ts';
import type { RegionList } from './useRegionList.ts';
import { formatScalar } from '../valueFormat.ts';
import type { RegionChoice } from './regionChoice.ts';
import { moveRegion } from './regionChoice.ts';

export const REGION_ADD_KEY = '__add-region';

export type RegionHitPart = 'add' | 'remove' | 'grip';

export interface RegionRowRef {
  readonly key: string;
  readonly id: number;
  readonly label: string;
  readonly picked: boolean;
  readonly top: number;
  readonly height: number;
  readonly active: boolean;
}

export interface RegionRowsField {
  readonly rows: readonly RegionRowRef[];
  readonly add: { readonly top: number; readonly height: number } | null;
  readonly nameLeft: number;
  readonly active?: ActiveRegions;
}

export function regionHitKey(part: RegionHitPart, rowKey: string): string {
  return `region-${part}:${rowKey}`;
}

export function parseRegionHitKey(hitKey: string | null): { readonly part: RegionHitPart; readonly key: string } | null {
  if (!hitKey?.startsWith('region-')) return null;
  const sep = hitKey.indexOf(':');
  const part = hitKey.slice(7, sep);
  if (sep < 0 || (part !== 'add' && part !== 'remove' && part !== 'grip')) return null;
  return { part, key: hitKey.slice(sep + 1) };
}

export function regionRowOf(field: RegionRowsField | undefined, key: string | null): RegionRowRef | null {
  if (!field || key === null) return null;
  return field.rows.find(row => row.key === key) ?? null;
}

export function keyboardMove(row: RegionRowRef, delta: -1 | 1, choice: RegionChoice): RegionChoice | null {
  const at = choice.picks.indexOf(row.id);
  if (at < 0) return delta < 0 ? moveRegion(choice, row.id, choice.picks.length) : null;
  const to = at + delta;
  return to < 0 || to >= choice.picks.length ? null : moveRegion(choice, row.id, to);
}

export interface RegionDrop {
  readonly to: number;
  readonly lineY: number;
}

export function regionDropAt(field: RegionRowsField, dragged: RegionRowRef, y: number): RegionDrop | null {
  const picks = field.rows.filter(row => row.picked && row.key !== dragged.key);
  const first = field.rows[0];
  if (!first) return null;
  const lastPick = picks.at(-1);
  const bottom = lastPick ? lastPick.top + lastPick.height : first.top;
  if (!dragged.picked && y > bottom + first.height / 2) return null;
  let to = 0;
  while (to < picks.length && y > picks[to]!.top + picks[to]!.height / 2) to++;
  const lineY = to < picks.length ? picks[to]!.top : bottom;
  return { to, lineY };
}

export type RegionCommand =
  | { readonly op: 'open'; readonly x: number; readonly y: number; readonly rows: readonly RegionRowRef[]; readonly active?: ActiveRegions }
  | { readonly op: 'remove'; readonly row: RegionRowRef }
  | { readonly op: 'step'; readonly row: RegionRowRef; readonly delta: -1 | 1 }
  | { readonly op: 'drop'; readonly row: RegionRowRef; readonly to: number };

export const PICKED_REGIONS_HEADING = 'Your regions';
export const ADD_REGION_LABEL = 'Add region';

export function mostActiveHeading(subject: string | null): string {
  return subject ? `Most active for ${subject}` : 'Most active';
}

export function regionLanes(subject: string | null, automaticHeading = mostActiveHeading(subject)): { readonly pickedHeading: string; readonly automaticHeading: string; readonly addLabel: string } {
  return { pickedHeading: PICKED_REGIONS_HEADING, automaticHeading, addLabel: ADD_REGION_LABEL };
}

export function countrySubject(readView: BoundedReadView, country: string | null): string | null {
  if (!country) return null;
  const name = readView.who('country', country).name;
  return name.status === 'ready' ? name.value : null;
}

export interface RegionNouns {
  readonly one: string;
  readonly many: string;
  readonly none: string;
}

export function validNoRegions(value: unknown): value is NoRegions | null {
  if (value === null) return true;
  if (!value || typeof value !== 'object') return false;
  const empty = value as Record<string, unknown>;
  const count = (item: unknown) => Number.isSafeInteger(item) && (item as number) >= 0;
  return empty.kind === 'none' || (empty.kind === 'unread' && typeof empty.reason === 'string')
    || (empty.kind === 'filtered' && count(empty.hidden) && count(empty.outside));
}

export function validActiveRegions(value: unknown): value is ActiveRegions {
  if (!value || typeof value !== 'object') return false;
  const active = value as Record<string, unknown>;
  return Array.isArray(active.ids) && active.ids.every(id => Number.isSafeInteger(id) && (id as number) >= 0)
    && (active.missing === null || typeof active.missing === 'string');
}

export function noRegionsBody(empty: NoRegions, nouns: RegionNouns): { readonly empty: string; readonly emptyKind: 'empty' | 'unavailable' } {
  if (empty.kind === 'unread') return { empty: /[.!?]$/u.test(empty.reason) ? empty.reason : `${empty.reason}.`, emptyKind: 'unavailable' };
  if (empty.kind === 'none') return { empty: nouns.none, emptyKind: 'empty' };
  const total = empty.hidden + empty.outside;
  const lead = total === 1 ? `The only ${nouns.one} at this date is` : `All ${formatScalar(total, 'integer')} ${nouns.many} at this date are`;
  const where = empty.outside === 0 ? 'removed from this chart'
    : empty.hidden === 0 ? 'outside the selected air theatres'
      : `removed from this chart (${formatScalar(empty.hidden, 'integer')}) or outside the selected air theatres (${formatScalar(empty.outside, 'integer')})`;
  return { empty: `${lead} ${where}.`, emptyKind: 'empty' };
}

export interface RegionInChart {
  readonly id: number;
  readonly label: string;
  readonly active?: boolean;
}

interface ChooserRow {
  readonly id: number;
  readonly label: string;
  readonly inChart: boolean;
}

interface ChooserGroup {
  readonly title: string;
  readonly rows: readonly ChooserRow[];
  readonly note?: string;
}

export function regionChooserGroups(list: RegionList, inChart: readonly RegionInChart[], active: ActiveRegions | null, seaOnly: boolean, query: string): readonly ChooserGroup[] {
  const needle = query.trim().toLocaleLowerCase();
  const matches = (label: string) => !needle || label.toLocaleLowerCase().includes(needle);
  const charted = new Set(inChart.map(row => row.id));
  const names = new Map(list.kind === 'ready' ? list.regions.map(region => [region.id, region.label] as const) : []);
  const activeRows: ChooserRow[] = inChart.filter(row => row.active).map(row => ({ id: row.id, label: row.label, inChart: true }));
  for (const id of active?.ids ?? []) {
    if (!charted.has(id)) activeRows.push({ id, label: names.get(id) ?? `Region ${id}`, inChart: false });
  }
  const regions = list.kind === 'ready' ? list.regions.filter(region => matches(region.label)) : [];
  const listed = (sea: boolean) => regions.filter(region => region.sea === sea).map(region => ({ id: region.id, label: region.label, inChart: charted.has(region.id) }));
  const groups: ChooserGroup[] = [{
    title: active ? 'Active now' : 'Active in this chart',
    rows: activeRows.filter(row => matches(row.label)),
    ...(active?.missing && !needle ? { note: active.missing } : {}),
  }];
  if (!seaOnly) groups.push({ title: 'Land regions', rows: listed(false) });
  groups.push({ title: 'Sea regions', rows: listed(true) });
  return groups.filter(group => group.rows.length > 0 || group.note);
}
