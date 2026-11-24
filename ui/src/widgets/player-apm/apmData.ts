import type { ApmReply, ApmValue } from '../../generated/dashboard';

const count = (value: unknown) => value === null || (typeof value === 'string' && /^\d+$/.test(value));
const rate = (value: unknown) => value === null || (typeof value === 'number' && Number.isFinite(value) && value >= 0);
const validValue = (value: ApmValue) => value && rate(value.apm) && rate(value.confirmedApm) && count(value.orders)
  && count(value.pausedOrders) && typeof value.complete === 'boolean' && typeof value.gaps === 'boolean' && rate(value.unpausedMinutes);
const identityOrder = new Intl.Collator('en', { numeric: true });

export function validateApm(value: unknown): ApmReply {
  const reply = value as ApmReply;
  if (!reply || typeof reply.available !== 'boolean' || !Array.isArray(reply.warnings) || !reply.warnings.every(warning => typeof warning === 'string')
    || !Array.isArray(reply.players) || !reply.players.every(player => player && typeof player.id === 'string'
      && (player.name === null || typeof player.name === 'string')
      && (player.nameMissing === null || typeof player.nameMissing === 'string')
      && Array.isArray(player.tags) && player.tags.every(tag => typeof tag === 'string'))
    || !Array.isArray(reply.series) || !reply.series.every(series => series && typeof series.id === 'string'
      && (series.name === null || typeof series.name === 'string')
      && (series.nameMissing === null || typeof series.nameMissing === 'string')
      && Number.isSafeInteger(series.hour)
      && (series.tag === null || typeof series.tag === 'string') && validValue(series.summary) && Array.isArray(series.points)
      && series.points.every((point, index) => Number.isSafeInteger(point.firstHour) && Number.isSafeInteger(point.lastHour)
        && point.firstHour <= point.lastHour && (!index || series.points[index - 1].lastHour < point.firstHour) && validValue(point.value))))
    throw new Error('The eAPM reply is malformed.');
  return reply;
}

export function playerNames(
  rows: readonly { id: string; name: string | null }[],
  registry?: readonly { id: string; name: string | null }[],
): Map<string, string> {
  const dated = new Map<string, string>();
  const seen = new Set<string>();
  const all: { id: string; name: string }[] = [];
  for (const row of rows) {
    if (row.name !== null && row.name.trim() !== '') {
      dated.set(row.id, row.name);
      if (!seen.has(row.id)) {
        seen.add(row.id);
        all.push({ id: row.id, name: row.name });
      }
    }
  }
  if (registry) {
    for (const row of registry) {
      if (row.name !== null && row.name.trim() !== '' && !seen.has(row.id)) {
        seen.add(row.id);
        all.push({ id: row.id, name: row.name });
      }
    }
  }
  const shared = new Map<string, string[]>();
  for (const row of all.sort((a, b) => identityOrder.compare(a.id, b.id))) {
    shared.set(row.name, [...shared.get(row.name) ?? [], row.id]);
  }
  const out = new Map<string, string>();
  for (const [id, name] of dated) {
    const ids = shared.get(name)!;
    out.set(id, ids.length > 1 ? `${name} (${ids.indexOf(id) + 1})` : name);
  }
  return out;
}

export const apmText = (value: number | null | undefined) => value == null ? 'Unknown' : value.toLocaleString(undefined, { maximumFractionDigits: 1 });
export const APM_EXPLANATION = 'Effective actions per minute (eAPM): accepted player-issued gameplay orders per unpaused real minute. Paused orders and paused time never affect eAPM. Camera movement, selections and unsaved interface edits do not count. Automatic battle-plan execution does not add orders.';
export const GAPS_NOTE = 'Part of this period was not recorded; its orders and minutes are both left out.';
function rateNote(value: ApmValue): string {
  if (!value.complete) return 'Player orders or unpaused time were not fully recorded for this period; the rate is unknown.';
  if (value.unpausedMinutes === 0) return 'No unpaused time was observed in this period.';
  if (value.apm === null) return `It is unclear whether some orders count toward eAPM; the rate is unknown. Confirmed orders: ${apmText(value.confirmedApm)} per unpaused minute.`;
  return APM_EXPLANATION;
}
export function valueNote(value: ApmValue): string {
  return value.gaps ? `${rateNote(value)} ${GAPS_NOTE}` : rateNote(value);
}
