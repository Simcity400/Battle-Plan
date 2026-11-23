import { runAt } from '../../charts/series.ts';
import type { Run } from '../../charts/series';
import type { ApmReply, ApmSeries, ApmValue } from '../../generated/dashboard';

const count = (value: unknown) => value === null || (typeof value === 'string' && /^\d+$/.test(value));
const rate = (value: unknown) => value === null || (typeof value === 'number' && Number.isFinite(value) && value >= 0);
const validValue = (value: ApmValue) => value && rate(value.apm) && rate(value.confirmedApm) && count(value.orders)
  && count(value.pausedOrders) && typeof value.complete === 'boolean' && typeof value.gaps === 'boolean' && rate(value.unpausedMinutes);
const identityOrder = new Intl.Collator('en', { numeric: true });

export function validateApm(value: unknown): ApmReply {
  const reply = value as ApmReply;
  if (!reply || typeof reply.available !== 'boolean' || !Array.isArray(reply.warnings) || !reply.warnings.every(warning => typeof warning === 'string')
    || !Array.isArray(reply.players) || !reply.players.every(player => player && typeof player.id === 'string' && typeof player.name === 'string'
      && Array.isArray(player.tags) && player.tags.every(tag => typeof tag === 'string'))
    || !Array.isArray(reply.series) || !reply.series.every(series => series && typeof series.id === 'string' && typeof series.name === 'string'
      && Number.isSafeInteger(series.hour)
      && (series.tag === null || typeof series.tag === 'string') && validValue(series.summary) && Array.isArray(series.points)
      && series.points.every((point, index) => Number.isSafeInteger(point.firstHour) && Number.isSafeInteger(point.lastHour)
        && point.firstHour <= point.lastHour && (!index || series.points[index - 1].lastHour < point.firstHour) && validValue(point.value))))
    throw new Error('The eAPM reply is malformed.');
  return reply;
}

export function apmRuns(series: ApmSeries, paused = false): Run[] {
  const runs: Run[] = [];
  let gap = true;
  for (const point of series.points) {
    const value = paused ? (point.value.pausedOrders === null ? null : Number(point.value.pausedOrders)) : point.value.apm;
    if (value === null) { gap = true; continue; }
    runs.push({ firstHour: point.firstHour, lastHour: point.lastHour, value, breakBefore: gap });
    gap = false;
  }
  return runs;
}

export function playerNames(rows: readonly { id: string; name: string }[], registry = rows): Map<string, string> {
  const known = new Map(registry.map(row => [row.id, row.name]));
  const named = rows.map(row => ({ id: row.id, name: known.get(row.id) ?? row.name }));
  const shared = new Map<string, string[]>();
  for (const row of [...registry, ...named.filter(row => !known.has(row.id))].sort((a, b) => identityOrder.compare(a.id, b.id))) {
    const name = known.get(row.id) ?? row.name;
    shared.set(name, [...shared.get(name) ?? [], row.id]);
  }
  return new Map(named.map(row => {
    const ids = shared.get(row.name)!;
    return [row.id, ids.length > 1 ? `${row.name} (${ids.indexOf(row.id) + 1})` : row.name];
  }));
}

export const apmText = (value: number | null | undefined) => value == null ? 'Unknown' : value.toLocaleString(undefined, { maximumFractionDigits: 1 });
export const countText = (value: string | null | undefined) => value == null ? 'Unknown' : BigInt(value).toLocaleString();
export const pointAt = (series: ApmSeries, hour: number) => runAt(series.points, Math.floor(hour));
export const APM_EXPLANATION = 'Effective actions per minute (eAPM): accepted player-issued gameplay orders per unpaused real minute. Paused orders and paused time never affect eAPM. Camera movement, selections and unsaved interface edits do not count. Automatic battle-plan execution does not add orders.';
export const GAPS_NOTE = 'Part of this period was not recorded; its orders and minutes are both left out.';
export const apmValueText = (value: ApmValue | undefined) => value?.gaps && value.apm !== null ? `${apmText(value.apm)}*` : apmText(value?.apm);
function rateNote(value: ApmValue): string {
  if (!value.complete) return 'Player orders or unpaused time were not fully recorded for this period; the rate is unknown.';
  if (value.unpausedMinutes === 0) return 'No unpaused time was observed in this period.';
  if (value.apm === null) return `It is unclear whether some orders count toward eAPM; the rate is unknown. Confirmed orders: ${apmText(value.confirmedApm)} per unpaused minute.`;
  return APM_EXPLANATION;
}
export function valueNote(value: ApmValue): string {
  return value.gaps ? `${rateNote(value)} ${GAPS_NOTE}` : rateNote(value);
}
