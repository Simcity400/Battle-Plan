import { countryNameDetails } from '../../charts/tooltip.ts';
import type { PoliticsStateReply, StateExile } from '../../generated/dashboard';
import type { PoliticsStateAt } from '../../dashboard/politicsHistory';
import type { TooltipContent } from '../../charts/tooltip';
import type { SortValue } from '../../charts/table/logic';
import { dayLabel } from '../../charts/draw.ts';
import { liveKey } from '../../charts/live.ts';
import type { LiveValues } from '../../charts/live';
import { usNumber } from '../../charts/numberFormat.ts';
import { object, opaqueTag } from '../../dashboard/validation.ts';

const optionalText = (value: unknown) => value === null || typeof value === 'string';

/** `politics-state` at an hour, exiles view: every government in exile the hour shows, its numbers live cells. */
export function validateExiles(value: unknown): PoliticsStateReply {
  if (!object(value) || typeof value.branchId !== 'string' || value.view !== 'exiles' || !Array.isArray(value.exiles)
    || !value.exiles.every(exile => object(exile) && typeof exile.id === 'string' && opaqueTag(exile.tag) && (exile.host === null || opaqueTag(exile.host)))
    || !Array.isArray(value.unavailable) || !value.unavailable.every(entry => object(entry) && typeof entry.label === 'string' && optionalText(entry.reason))
    || !Array.isArray(value.warnings) || value.warnings.some(warning => typeof warning !== 'string'))
    throw new Error('The governments in exile reply is malformed.');
  return value as unknown as PoliticsStateReply;
}

/** One government in exile at the shown hour: `key` is its raw country id, the row of its live cells. */
export interface ExileRow {
  key: string; tag: string;
  /** The host's tag at the hour; null when the host has none recorded. */
  host: string | null;
}

/** An exile's numbers at the hour, from the live cells; null where not recorded, `why` naming the reason. */
export interface ExileNumbers {
  /** Legitimacy, percent. */
  legitimacy: number | null;
  manpower: number | null; cap: number | null; leaders: number | null;
  /** Crews of the wings flying for it in exile. */
  air: number | null;
  /** When the exile's resources were last read. */
  readHour: number | null;
  why: (cell: ExileCell) => string;
}

/** The live cells of an exile's row. */
export type ExileCell = 'legitimacy' | 'manpower' | 'cap' | 'leaders' | 'deployedAir' | 'readHour';

export function exileNumbers(values: LiveValues<number>, key: string): ExileNumbers {
  const read = (cell: ExileCell) => values.get(liveKey(key, cell)) ?? null;
  return { legitimacy: read('legitimacy'), manpower: read('manpower'), cap: read('cap'), leaders: read('leaders'), air: read('deployedAir'),
    readHour: read('readHour'), why: cell => values.missing(liveKey(key, cell)) };
}

/** The table's columns by their keys, each the live cell it paints and sorts by. */
export const EXILE_COLUMNS: Readonly<Record<'legitimacy' | 'manpower' | 'air' | 'leaders', ExileCell>> = {
  legitimacy: 'legitimacy', manpower: 'manpower', air: 'deployedAir', leaders: 'leaders',
};

/** The table's live source: each numeric column's cell under its column's key. */
export function exileTableValues(values: LiveValues<number>): LiveValues<SortValue> {
  const cell = (key: string) => {
    const at = key.lastIndexOf('\u0000'), column = key.slice(at + 1) as keyof typeof EXILE_COLUMNS;
    return EXILE_COLUMNS[column] ? liveKey(key.slice(0, at), EXILE_COLUMNS[column]) : key;
  };
  return { get: key => values.get(cell(key)), missing: key => values.missing(cell(key)) };
}
export interface ExilesView {
  rows: ExileRow[];
  /** Exiles whose host has no tag at the hour. */
  unknownHosts: number;
  /** Why an exile left out may be a chosen country's: the choice's membership not recorded at the hour. */
  membership: string | null;
}

/** Every government in exile in `reply`, kept when it or its host passes `matches` at the reply's hour; one left out
 * where `unrecorded` gives the choice's membership as not recorded says so. */
export function exilesAt(reply: Pick<PoliticsStateAt, 'exiles' | 'hour'>, matches: (tag: string, hour: number) => boolean,
  unrecorded: (hour: number) => string | null = () => null): ExilesView {
  const rows: ExileRow[] = [];
  let unknownHosts = 0, membership: string | null = null;
  for (const exile of reply.exiles as StateExile[]) {
    if (!matches(exile.tag, reply.hour) && !(exile.host !== null && matches(exile.host, reply.hour))) { membership ??= unrecorded(reply.hour); continue; }
    if (exile.host === null) unknownHosts++;
    rows.push({ key: exile.id, tag: exile.tag, host: exile.host });
  }
  return { rows, unknownHosts, membership };
}

/** Which exiles and hosts the table shows: a followed reply renders only when it changes; their numbers are painted. */
export const exileStructure = (reply: PoliticsStateReply): string => JSON.stringify([reply.exiles, reply.unavailable, reply.warnings]);

/** Legitimacy to two decimals, as the exile cards read it. */
export const legitimacyText = (percent: number): string => `${percent.toFixed(2)}%`;
/** Legitimacy as a share of its full 100. */
export const legitimacyShare = (percent: number): number => percent / 100;
export const countText = (value: number | null): string => value === null ? '—' : usNumber.format(value);
export const manpowerText = (row: Pick<ExileNumbers, 'manpower' | 'cap'>): string => `${countText(row.manpower)} / ${countText(row.cap)}`;
/** The supplying read's date, when it is older than the shown hour. */
export const readNote = (row: Pick<ExileNumbers, 'readHour'>, hour: number): string | null =>
  row.readHour !== null && row.readHour < hour ? `Exile resources last read ${dayLabel(row.readHour)}.` : null;

type Label = { nameFailure?: Error; nameDetail?: string; name: string; flag?: string; flagReason?: string; flagDetail?: string; flagPending?: boolean };
/** The row's facts in the exile cards' wording. */
export function exileTooltip(numbers: ExileNumbers, hour: number, exile: Label, host: Label | null): TooltipContent {
  const notRecorded = 'Not recorded';
  const number = (value: number | null, cell: ExileCell) => value === null ? numbers.why(cell) || notRecorded : usNumber.format(value);
  const note = readNote(numbers, hour);
  return { ...countryNameDetails(exile), title: exile.name, countryFlag: { flag: exile.flag, flagReason: exile.flagReason, ...(exile.flagDetail !== undefined ? { flagDetail: exile.flagDetail } : {}), flagPending: exile.flagPending }, rows: [
    host === null ? { label: 'Host', value: notRecorded }
      : { ...countryNameDetails(host), label: 'Host', value: host.name, countryFlag: { flag: host.flag, flagReason: host.flagReason, ...(host.flagDetail !== undefined ? { flagDetail: host.flagDetail } : {}), flagPending: host.flagPending } },
    { label: 'Legitimacy', value: numbers.legitimacy === null ? numbers.why('legitimacy') || notRecorded : legitimacyText(numbers.legitimacy) },
    { label: 'Exile manpower', value: number(numbers.manpower, 'manpower') },
    { label: 'Exile manpower cap', value: number(numbers.cap, 'cap') },
    { label: 'Deployed air crews', value: number(numbers.air, 'deployedAir') },
    { label: 'Army leaders recruited', value: number(numbers.leaders, 'leaders') },
  ], ...note ? { note } : {} };
}

/** Counted diagnostics behind what the table cannot show: the reply's own, and hosts with no tag. */
export function exileWarnings(reply: Pick<PoliticsStateReply, 'warnings'>, view: ExilesView | null): string[] {
  return view?.unknownHosts ? [...reply.warnings, `${view.unknownHosts} refused exile host identity: the host has no tag at this hour`] : reply.warnings;
}
