import type { LogRow } from '../../charts/log/logData';
import { newestFirst } from '../../charts/log/logData.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { TooltipContent } from '../../charts/tooltip';
import { drawableIcon } from '../../dashboard/artwork.ts';
import { campaignToken } from '../../dashboard/chosen.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { isOrderedSpans } from '../../charts/series.ts';
import { int, object } from '../../dashboard/validation.ts';
import { contentTitleHint } from '../../dashboard/contentTitles.ts';
import type { ShipClassInfo, ShipsSunkManifestReply, ShipsSunkReply, ShipsSunkRow } from '../../generated/dashboard';

export type CountryRole = 'victim' | 'sinker' | 'both';
const text = (value: unknown) => value === null || typeof value === 'string';
const optionalInt = (value: unknown) => value === null || int(value);
const validAttribution = (value: unknown) => {
  if (!object(value) || typeof value.kind !== 'string') return false;
  if (value.kind === 'cause') return typeof value.text === 'string' && typeof value.assisted === 'boolean';
  if (value.kind === 'ship') return typeof value.name === 'string' && text(value.classKey) && text(value.className) && typeof value.assisted === 'boolean';
  if (value.kind === 'airWing') return text(value.carrierName) && text(value.carrierClassKey) && text(value.carrierClassName) && typeof value.assisted === 'boolean';
  if (value.kind === 'notRecorded') return typeof value.reason === 'string';
  return false;
};
const validBattle = (value: unknown) => {
  if (!object(value) || typeof value.kind !== 'string') return false;
  if (value.kind === 'linked') return typeof value.id === 'string' && value.id.length > 0 && text(value.liveId) && text(value.resultId) && optionalInt(value.firstHour) && optionalInt(value.resultHour);
  if (value.kind === 'notRecorded') return typeof value.reason === 'string';
  return false;
};

export function validateShipsSunkReply(data: unknown): ShipsSunkReply {
  const reply = data as ShipsSunkReply;
  const ok = object(reply) && typeof reply.branchId === 'string' && typeof reply.available === 'boolean'
    && int(reply.firstHour) && int(reply.lastHour) && reply.firstHour <= reply.lastHour && optionalInt(reply.firstRead)
    && isOrderedSpans(reply.unread) && Array.isArray(reply.warnings) && reply.warnings.every(warning => typeof warning === 'string')
    && Array.isArray(reply.classes) && reply.classes.every(type => object(type) && int(type.token) && type.token >= 0
      && text(type.key) && text(type.name) && text(type.icon))
    && Array.isArray(reply.recordedNames) && reply.recordedNames.every(name => typeof name === 'string')
    && Array.isArray(reply.places) && reply.places.every(place => typeof place === 'string')
    && Array.isArray(reply.rows) && reply.rows.every(row => object(row) && int(row.sinkHour) && int(row.knownAt)
      && reply.firstHour <= row.knownAt && row.knownAt <= reply.lastHour
      && row.sinkHour <= row.knownAt && int(row.classIndex) && row.classIndex >= 0 && row.classIndex < reply.classes.length
      && (row.recordedNameIndex === null || (int(row.recordedNameIndex) && row.recordedNameIndex >= 0 && row.recordedNameIndex < reply.recordedNames.length))
      && [row.name, row.victim, row.attacker, row.attributionName].every(text)
      && [[row.victim, row.victimAsOf], [row.attacker, row.attackerAsOf]].every(([tag, hour]) => hour === null
        || (tag !== null && int(hour) && hour === row.knownAt && row.sinkHour < hour))
      && (row.placeIndex === null || (int(row.placeIndex) && row.placeIndex >= 0 && row.placeIndex < reply.places.length))
      && ['hostile', 'assisted', 'cause'].includes(row.cause)
      && typeof row.assistedKill === 'boolean'
      && validAttribution(row.sinkingCause) && validBattle(row.battle));
  if (!ok) throw new Error('The ships sunk reply is malformed.');
  for (const type of reply.classes) type.icon = drawableIcon(type.icon);
  return reply;
}

export function shipClassKey(reply: ShipsSunkReply, type: ShipClassInfo, campaignKey: string): string {
  return type.key ?? campaignToken(campaignKey, `${reply.branchId}:${type.token}`);
}

export function shipClassName(reply: ShipsSunkReply, row: ShipsSunkRow): string {
  const type = reply.classes[row.classIndex];
  const recorded = row.recordedNameIndex === null ? null : reply.recordedNames[row.recordedNameIndex];
  return type.name ?? recorded ?? 'Class name unavailable';
}

export function manifestClassChoices(manifest: ShipsSunkManifestReply | undefined, campaignKey: string): { value: string; label: string; icon: string | null; hint?: string }[] {
  if (!manifest) return [];
  const choices = manifest.classes.map(type => ({ value: type.key ?? campaignToken(campaignKey, `${manifest.branchId}:${type.token}`),
    label: type.name ?? 'Class name unavailable', icon: type.icon, hint: contentTitleHint(type.key ?? type.token) }));
  return [...new Map(choices.map(choice => [choice.value, choice])).values()].sort((a, b) => a.label.localeCompare(b.label));
}

const shipOrders = new WeakMap<ShipsSunkReply, ReadonlyArray<LogRow<ShipsSunkRow>>>();
export function shipLogOrder(reply: ShipsSunkReply): ReadonlyArray<LogRow<ShipsSunkRow>> {
  let rows = shipOrders.get(reply);
  if (!rows) { rows = newestFirst(reply.rows.map((row, index) => ({ key: String(index), hour: row.sinkHour, item: row }))); shipOrders.set(reply, rows); }
  return rows;
}

export const shipCountryHour = (row: ShipsSunkRow, role: 'victim' | 'sinker') =>
  (role === 'victim' ? row.victimAsOf : row.attackerAsOf) ?? row.sinkHour;

export function shipCountryLabels(row: ShipsSunkRow, labelAt: (hour: number) => CountryLabel): {
  victim: ReturnType<CountryLabel>; attacker: ReturnType<CountryLabel>;
} {
  return {
    victim: row.victim === null ? { name: 'Not recorded', flag: undefined } : labelAt(shipCountryHour(row, 'victim'))(row.victim),
    attacker: row.attacker === null ? { name: 'Not recorded', flag: undefined } : labelAt(shipCountryHour(row, 'sinker'))(row.attacker),
  };
}

export function shipLogRows(reply: ShipsSunkReply, hour: number, classes: readonly string[], role: CountryRole,
  country: (tag: string | null, hour: number) => boolean, from: number | null,
  ordered: readonly LogRow<ShipsSunkRow>[] = shipLogOrder(reply), campaignKey = ''): ReadonlyArray<LogRow<ShipsSunkRow>> {
  const selected = new Set(classes);
  const rows: LogRow<ShipsSunkRow>[] = [];
  let low = 0;
  let high = ordered.length;
  while (low < high) {
    const middle = (low + high) >>> 1;
    if (ordered[middle].hour > hour) low = middle + 1; else high = middle;
  }
  for (let index = low; index < ordered.length; index++) {
    const log = ordered[index];
    const row = log.item;
    if (from !== null && row.sinkHour < from) break;
    if (row.knownAt > hour) continue;
    if (selected.size && !selected.has(shipClassKey(reply, reply.classes[row.classIndex], campaignKey))) continue;
    const victim = role !== 'sinker' && country(row.victim, shipCountryHour(row, 'victim'));
    const sinker = role !== 'victim' && country(row.attacker, shipCountryHour(row, 'sinker'));
    if (!victim && !sinker) continue;
    rows.push(log);
  }
  return rows;
}

export const shipName = (row: ShipsSunkRow) => row.name ?? 'Not recorded';
export const shipPlace = (reply: ShipsSunkReply, row: ShipsSunkRow) => row.placeIndex === null ? 'Not recorded' : reply.places[row.placeIndex];
export function shipCauseLabel(row: ShipsSunkRow): string {
  if (row.cause === 'cause') return 'Non-hostile cause';
  if (row.assistedKill || row.cause === 'assisted') return 'Assisted kill';
  return 'Sunk';
}

export function shipAction(row: ShipsSunkRow, attacker: string): string {
  if (row.cause === 'cause') return `lost to ${row.attributionName ?? 'Not recorded'}`;
  if (row.assistedKill || row.cause === 'assisted') return `sunk with assistance from ${attacker}`;
  return `sunk by ${attacker}`;
}

export function shipDescription(reply: ShipsSunkReply, row: ShipsSunkRow, victim: string, attacker: string): string {
  const attribution = row.cause === 'cause' && row.attacker !== null ? `; attributed to ${attacker}` : '';
  return `${shipName(row)}, ${shipClassName(reply, row)}, ${victim}, ${shipAction(row, attacker)}${attribution} in ${shipPlace(reply, row)}, ${formatDateTime(row.sinkHour)}`;
}

export function shipTooltip(reply: ShipsSunkReply, row: ShipsSunkRow, victim: string, attacker: string): TooltipContent {
  return { title: shipName(row), icon: reply.classes[row.classIndex].icon ?? undefined, subtitle: formatDateTime(row.sinkHour), rows: [
    { label: 'Class', value: shipClassName(reply, row) },
    { label: 'Class key', value: reply.classes[row.classIndex].key ?? String(reply.classes[row.classIndex].token) },
    { label: 'Victim', value: row.victimAsOf === null ? victim : `${victim} (as of ${formatDateTime(row.victimAsOf)})` },
    { label: 'Credited country', value: row.attackerAsOf === null ? attacker : `${attacker} (as of ${formatDateTime(row.attackerAsOf)})` },
    { label: 'Cause', value: row.cause === 'cause' ? row.attributionName ?? 'Not recorded' : shipCauseLabel(row) },
    ...(row.cause === 'cause' && row.assistedKill ? [{ label: 'Assisted kill', value: 'Yes' }] : []),
    { label: 'Sea zone / region', value: shipPlace(reply, row) },
  ] };
}
