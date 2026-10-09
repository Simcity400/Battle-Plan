import type { ShipBattleTitle, ShipClassInfo, ShipsSunkBattleTitlesReply, ShipsSunkReply, ShipsSunkRow } from '../../generated/dashboard';
import type { LogRow } from '../../charts/log/logData';
import { logCountryRequests, newestFirst } from '../../charts/log/logData.ts';
import type { CountryLabel } from '../../dashboard/countryFlagsAt';
import type { TooltipContent } from '../../charts/tooltip';
import { drawableIcon } from '../../dashboard/artwork.ts';
import { campaignToken } from '../../dashboard/chosen.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import { isOrderedSpans } from '../../charts/series.ts';
import { int, object } from '../../dashboard/validation.ts';

export type CountryRole = 'victim' | 'sinker' | 'both';
const text = (value: unknown) => value === null || typeof value === 'string';
const optionalInt = (value: unknown) => value === null || int(value);

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
      && typeof row.assistedKill === 'boolean');
  if (!ok) throw new Error('The ships sunk reply is malformed.');
  for (const type of reply.classes) type.icon = drawableIcon(type.icon);
  return reply;
}

export function validateShipBattleTitlesReply(data: unknown): ShipsSunkBattleTitlesReply {
  const reply = data as ShipsSunkBattleTitlesReply;
  if (!object(reply) || !Array.isArray(reply.warnings) || !reply.warnings.every(warning => typeof warning === 'string')
    || !Array.isArray(reply.rows) || !reply.rows.every(row => object(row) && int(row.index) && row.index >= 0 && row.index <= 0xffffffff
      && int(row.hour) && text(row.name) && text(row.missing) && (row.name === null || row.missing === null))
    || new Set(reply.rows.map(row => row.index)).size !== reply.rows.length) throw new Error('The ships sunk battle-title reply is malformed.');
  return reply;
}

function currentRows(rows: readonly LogRow<ShipsSunkRow>[], reply: ShipsSunkReply | undefined) {
  return rows.filter(row => reply?.rows[Number(row.key)] === row.item);
}

/** Stable scan indices, only for drawn rows of the current source reply. */
export function shipBattleRequests(rows: readonly LogRow<ShipsSunkRow>[], reply: ShipsSunkReply | undefined): number[] {
  return [...new Set(currentRows(rows, reply).map(row => Number(row.key)))].sort((a, b) => a - b);
}

/** The native title is already formatted by content; keep its supplying date explicit. */
export function shipBattleLabel(row: ShipsSunkRow, battle: ShipBattleTitle | undefined): string | undefined {
  if (!battle || (battle.name === null && battle.missing === null)) return undefined;
  return battle.name === null ? 'Battle title not recorded' : `${battle.name}${battle.hour === row.sinkHour ? '' : ` (as of ${formatDateTime(battle.hour)})`}`;
}

/** Content keys survive other recordings; unresolved lexical tokens stay branch-specific. */
export function shipClassKey(reply: ShipsSunkReply, type: ShipClassInfo, campaignKey: string): string {
  return type.key ?? campaignToken(campaignKey, `${reply.branchId}:${type.token}`);
}

export function shipClassName(reply: ShipsSunkReply, row: ShipsSunkRow): string {
  const type = reply.classes[row.classIndex];
  const recorded = row.recordedNameIndex === null ? null : reply.recordedNames[row.recordedNameIndex];
  return type.name ?? (recorded ? `${recorded} (recorded class)`
    : type.key ? `${type.key} (class name not resolved)` : `Class token ${type.token} (name not recorded)`);
}

export function shipClassChoices(reply: ShipsSunkReply | undefined, campaignKey: string): { value: string; label: string; icon: string | null }[] {
  if (!reply) return [];
  const present = new Map<number, ShipsSunkRow>();
  for (const row of reply.rows) if (!present.has(row.classIndex)) present.set(row.classIndex, row);
  const choices = [...new Map([...present].map(([index, row]) => ({ value: shipClassKey(reply, reply.classes[index], campaignKey), label: shipClassName(reply, row), icon: reply.classes[index].icon })).map(choice => [choice.value, choice])).values()];
  const labels = new Map<string, number>();
  for (const choice of choices) labels.set(choice.label, (labels.get(choice.label) ?? 0) + 1);
  return choices.map(choice => ({ ...choice, label: (labels.get(choice.label) ?? 0) > 1 ? `${choice.label} (${choice.value})` : choice.label }))
    .sort((a, b) => a.label.localeCompare(b.label));
}

/** Immutable event order is built once per reply, not on every playback hour. */
export function shipLogOrder(reply: ShipsSunkReply): LogRow<ShipsSunkRow>[] {
  return newestFirst(reply.rows.map((row, index) => ({ key: String(index), hour: row.sinkHour, item: row })));
}

/** Event dates stay separate from an explicitly dated supplying-read country. */
export const shipCountryHour = (row: ShipsSunkRow, role: 'victim' | 'sinker') =>
  (role === 'victim' ? row.victimAsOf : row.attackerAsOf) ?? row.sinkHour;

export function shipCountryRequests(rows: readonly LogRow<ShipsSunkRow>[], reply: ShipsSunkReply | undefined) {
  return logCountryRequests(currentRows(rows, reply).flatMap(row => [
    { ...row, hour: shipCountryHour(row.item, 'victim'), item: row.item.victim },
    { ...row, hour: shipCountryHour(row.item, 'sinker'), item: row.item.attacker },
  ]), row => ({ hour: row.hour, tags: [row.item] }));
}

export function shipCountryLabels(row: ShipsSunkRow, labelAt: (hour: number) => CountryLabel): {
  victim: ReturnType<CountryLabel>; attacker: ReturnType<CountryLabel>;
} {
  return {
    victim: row.victim === null ? { name: 'Not recorded', flag: undefined } : labelAt(shipCountryHour(row, 'victim'))(row.victim),
    attacker: row.attacker === null ? { name: 'Not recorded', flag: undefined } : labelAt(shipCountryHour(row, 'sinker'))(row.attacker),
  };
}

/** Visibility starts at the supplying read; event dates still order and filter rows. */
export function shipLogRows(reply: ShipsSunkReply, hour: number, classes: readonly string[], role: CountryRole,
  country: (tag: string | null, hour: number) => boolean, from: number | null,
  ordered: readonly LogRow<ShipsSunkRow>[] = shipLogOrder(reply), campaignKey = ''): LogRow<ShipsSunkRow>[] {
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

export function shipDescription(reply: ShipsSunkReply, row: ShipsSunkRow, victim: string, attacker: string, battle?: ShipBattleTitle): string {
  const attribution = row.cause === 'cause' && row.attacker !== null ? `; attributed to ${attacker}` : '';
  const title = shipBattleLabel(row, battle);
  return `${shipName(row)}, ${shipClassName(reply, row)}, ${victim}, ${shipAction(row, attacker)}${attribution} in ${shipPlace(reply, row)}${title ? `, ${title}` : ''}, ${formatDateTime(row.sinkHour)}`;
}

export function shipTooltip(reply: ShipsSunkReply, row: ShipsSunkRow, victim: string, attacker: string,
  battle?: ShipBattleTitle, titleError?: string): TooltipContent {
  const title = shipBattleLabel(row, battle);
  return { title: shipName(row), icon: reply.classes[row.classIndex].icon ?? undefined, subtitle: formatDateTime(row.sinkHour), rows: [
    { label: 'Class', value: shipClassName(reply, row) },
    { label: 'Victim', value: row.victimAsOf === null ? victim : `${victim} (as of ${formatDateTime(row.victimAsOf)})` },
    { label: 'Attribution country', value: row.attackerAsOf === null ? attacker : `${attacker} (as of ${formatDateTime(row.attackerAsOf)})` },
    { label: 'Cause', value: row.cause === 'cause' ? row.attributionName ?? 'Not recorded' : shipCauseLabel(row) },
    { label: 'Assisted kill', value: row.assistedKill ? 'Yes' : 'No' },
    { label: 'Sea zone / region', value: shipPlace(reply, row) },
    ...(title ? [{ label: 'Battle', value: title }] : !battle && row.cause !== 'cause'
      ? [{ label: 'Battle', value: titleError ? 'Battle title unavailable' : 'Loading battle title' }] : []),
    ...(battle?.missing || (!battle && row.cause !== 'cause' && titleError) ? [{ label: 'Battle title unavailable', value: battle?.missing ?? titleError! }] : []),
    { label: 'First observed', value: formatDateTime(row.knownAt), divider: true },
  ], note: 'Dated by the engine’s recorded sinking hour. Battle names use the supplying read, shown as of that date when different; they do not reconstruct an unrecorded event-time name. Non-hostile causes are not enemy kills; assisted-kill records are marked separately.' };
}

export function shipCoverage(reply: ShipsSunkReply): string {
  return `Recording: ${formatDateTime(reply.firstHour)}–${formatDateTime(reply.lastHour)}. ${reply.firstRead === null
    ? 'No complete ships read.' : `First complete ships read: ${formatDateTime(reply.firstRead)}.`}${reply.unread.length
    ? ` ${reply.unread.length} unread span${reply.unread.length === 1 ? '' : 's'}; sinkings pruned before the next read may be missing.` : ''}`;
}
