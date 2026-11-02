import { countryNameDetails } from '../../charts/tooltip.ts';
import type { PeopleExile, PoliticsHistoryReply, PoliticsStateReply } from '../../generated/dashboard';
import type { TooltipContent } from '../../charts/tooltip';
import type { SortValue } from '../../charts/table/logic';
import { dayLabel } from '../../charts/draw.ts';
import { usNumber } from '../../charts/numberFormat.ts';
import { formatNativeFixed5 } from '../../charts/valueAxis.ts';
import { object, opaqueTag } from '../../dashboard/validation.ts';
import { fixed5Text, holderKey, integerAt, integerText, isHolderTags, isPoliticsRuns, tagAt, validatePoliticsHistory, valueAt } from '../../dashboard/politicsHistory.ts';

const holderId = (value: string) => /^\d{1,10}$/.test(value);

/** The `exiles` history: every exile's host holder, resources and supplying read, and each holder's tags. */
export function validateExilesReply(value: unknown): PoliticsHistoryReply {
  return validatePoliticsHistory(value, 'exiles', reply => Array.isArray(reply.exileHistory) && reply.exileHistory.every(entry =>
    object(entry) && isHolderTags(entry.tags) && isPoliticsRuns(entry.host, holderId) && isPoliticsRuns(entry.legitimacy, fixed5Text)
    && isPoliticsRuns(entry.manpower, integerText) && isPoliticsRuns(entry.cap, integerText) && isPoliticsRuns(entry.leaders, integerText)
    && isPoliticsRuns(entry.read, integerText))
    && object(reply.holders) && Object.entries(reply.holders).every(([id, tags]) => holderId(id) && isHolderTags(tags)));
}

/** The hour detail: deployed air crews of each exile at the shown hour. */
export function validateExileDetails(value: unknown, hour: number): PoliticsStateReply {
  const optional = (value: unknown) => value === null || typeof value === 'number' && Number.isSafeInteger(value);
  if (!object(value) || typeof value.branchId !== 'string' || value.hour !== hour || value.view !== 'exiles' || !Array.isArray(value.exiles)
    || !value.exiles.every(exile => object(exile) && opaqueTag(exile.tag) && optional(exile.deployedAir))
    || !Array.isArray(value.warnings) || value.warnings.some(warning => typeof warning !== 'string'))
    throw new Error('The exile details reply is malformed.');
  return value as unknown as PoliticsStateReply;
}

/** One government in exile at the shown hour. Null values were not recorded. */
export interface ExileRow {
  key: string; tag: string;
  /** The host's tag at the hour; null when the host has none recorded. */
  host: string | null;
  /** Exact fixed5 legitimacy. */
  legitimacy: string | null;
  manpower: number | null; cap: number | null; leaders: number | null;
  /** When the exile's resources were last read. */
  readHour: number | null;
}
export interface ExilesView {
  rows: ExileRow[];
  /** Exiles whose host has no tag at the hour. */
  unknownHosts: number;
}

/** Every government in exile at `hour` (a host run covers it), kept when it or its host passes `matches`. */
export function exilesAt(reply: Pick<PoliticsHistoryReply, 'exileHistory' | 'holders'>, hour: number, matches: (tag: string, hour: number) => boolean): ExilesView {
  const rows: ExileRow[] = [];
  const keys = new Set<string>();
  let unknownHosts = 0;
  for (const entry of reply.exileHistory) {
    const hostId = valueAt(entry.host, hour);
    if (hostId === null) continue;
    const tag = tagAt(entry.tags, hour);
    if (tag === null) continue;
    const hostTags = reply.holders[hostId];
    const host = hostTags ? tagAt(hostTags, hour) : null;
    if (!matches(tag, hour) && !(host !== null && matches(host, hour))) continue;
    if (host === null) unknownHosts++;
    let key = holderKey(entry.tags)!;
    if (keys.has(key)) key = `${key}#${rows.length}`;
    keys.add(key);
    rows.push({ key, tag, host, legitimacy: valueAt(entry.legitimacy, hour), manpower: integerAt(entry.manpower, hour),
      cap: integerAt(entry.cap, hour), leaders: integerAt(entry.leaders, hour), readHour: integerAt(entry.read, hour) });
  }
  return { rows, unknownHosts };
}

/** Deployed air crews by exile tag, from the hour detail. */
export function deployedAirByTag(reply: Pick<PoliticsStateReply, 'exiles'>): Map<string, number | null> {
  return new Map(reply.exiles.map((exile: Pick<PeopleExile, 'tag' | 'deployedAir'>) => [exile.tag, exile.deployedAir]));
}

/** Exact percentage, as the exile cards read legitimacy. */
export const legitimacyText = (raw: string): string => `${formatNativeFixed5(raw, 2)}%`;
/** Legitimacy as a share of its full 100. */
export const legitimacyShare = (raw: string): number => Number(BigInt(raw)) / 10_000_000;
export const countText = (value: number | null): string => value === null ? '—' : usNumber.format(value);
export const manpowerText = (row: Pick<ExileRow, 'manpower' | 'cap'>): string => `${countText(row.manpower)} / ${countText(row.cap)}`;
/** The supplying read's date, when it is older than the shown hour. */
export const readNote = (row: Pick<ExileRow, 'readHour'>, hour: number): string | null =>
  row.readHour !== null && row.readHour < hour ? `Exile resources last read ${dayLabel(row.readHour)}.` : null;

/** Exact sort operands of the columns other than Exile and Host; air crews come from the hour detail. */
export const EXILE_VALUES: Readonly<Record<'legitimacy' | 'manpower' | 'leaders', (row: ExileRow) => SortValue>> = {
  legitimacy: row => row.legitimacy === null ? null : BigInt(row.legitimacy),
  manpower: row => row.manpower,
  leaders: row => row.leaders,
};

type Label = { nameFailure?: Error; nameDetail?: string; name: string; flag?: string; flagReason?: string; flagDetail?: string; flagPending?: boolean };
/** The row's facts in the exile cards' wording. `air` is undefined while the hour detail loads. */
export function exileTooltip(row: ExileRow, hour: number, exile: Label, host: Label | null, air: number | null | undefined): TooltipContent {
  const notRecorded = 'Not recorded';
  const number = (value: number | null) => value === null ? notRecorded : usNumber.format(value);
  const note = readNote(row, hour);
  return { ...countryNameDetails(exile), title: exile.name, countryFlag: { flag: exile.flag, flagReason: exile.flagReason, ...(exile.flagDetail !== undefined ? { flagDetail: exile.flagDetail } : {}), flagPending: exile.flagPending }, rows: [
    host === null ? { label: 'Host', value: notRecorded }
      : { ...countryNameDetails(host), label: 'Host', value: host.name, countryFlag: { flag: host.flag, flagReason: host.flagReason, ...(host.flagDetail !== undefined ? { flagDetail: host.flagDetail } : {}), flagPending: host.flagPending } },
    { label: 'Legitimacy', value: row.legitimacy === null ? notRecorded : legitimacyText(row.legitimacy) },
    { label: 'Exile manpower', value: number(row.manpower) },
    { label: 'Exile manpower cap', value: number(row.cap) },
    ...air === undefined ? [] : [{ label: 'Deployed air crews', value: number(air) }],
    { label: 'Army leaders recruited', value: number(row.leaders) },
  ], ...note ? { note } : {} };
}

/** Counted diagnostics behind what the table cannot show. */
export function exileWarnings(reply: Pick<PoliticsHistoryReply, 'warnings'>, view: ExilesView | null, details: readonly string[]): string[] {
  const own = view?.unknownHosts ? [`${view.unknownHosts} refused exile host identity: the host has no tag at this hour`] : [];
  return own.length || details.length ? [...reply.warnings, ...own, ...details] : reply.warnings;
}
