import { liveKey } from '../../charts/live.ts';
import type { Live, LiveValues } from '../../charts/live';
import { equipmentOf, FIXED5, FUEL_QUANTITY, fixedText, SHARE } from '../../dashboard/market.ts';
import type { MarketAt } from '../../dashboard/market';
import type { EndedLease, EndedLeaseTerms, LeaseTerms, LendLeaseHistoryReply, MarketItem } from '../../generated/dashboard';

export type Direction = 'sent' | 'received';
export type AgreementTerms = LeaseTerms | EndedLeaseTerms;

export interface AgreementItem {
  variant: string | null;
  name: string | null;
  icon: string | null;
  missing: string | null;
  schedule: string;
  scheduleMissing: string | null;
  delivered: bigint | null;
  deliveredMissing: string | null;
  lost: bigint | null;
  lostMissing: string | null;
}

export interface AgreementRow {
  key: string;
  terms: AgreementTerms;
  at: number;
  ended: boolean;
  from: string | null;
  partner: string | null;
  partnerMissing: string | null;
  giver: string | null;
  giverMissing: string | null;
  receiver: string | null;
  receiverMissing: string | null;
  fuel: boolean;
  items: AgreementItem[];
  since: number | null;
  sinceMissing: string | null;
  last: number | null;
  lastMissing: string | null;
}

const units = (raw: bigint) => fixedText(raw, FIXED5);
const share = (raw: bigint) => `${fixedText(raw, SHARE)}%`;
const fuelText = (raw: bigint) => fixedText(raw, FUEL_QUANTITY);
const byVariant = (list: readonly MarketItem[] | null) => new Map((list ?? []).map(item => [item.variant, BigInt(item.amountRaw)]));
const startHourOf = (value: number | null, missing?: string): { value: number | null; missing: string | null } =>
  value === null ? { value: null, missing: missing ?? 'Start date not recorded' } : { value, missing: null };
const deliveryHourOf = (value: number | null, missing?: string): { value: number | null; missing: string | null } =>
  value === null ? { value: null, missing: missing ?? 'Last delivery date not recorded' } : value > 0 ? { value, missing: null } : { value: null, missing: null };
const amountBigInt = (valueRaw: string | null, missing?: string): { value: bigint | null; missing: string | null } =>
  valueRaw === null ? { value: null, missing: missing ?? 'Amount not recorded' } : { value: BigInt(valueRaw), missing: null };
const joinMissing = (reasons: readonly (string | null | undefined)[]): string | null => {
  const unique = [...new Set(reasons.filter((reason): reason is string => typeof reason === 'string' && reason.length > 0))];
  return unique.length ? unique.join('; ') : null;
};

type Named = ReturnType<typeof equipmentOf>;

function rowsOf(terms: AgreementTerms, at: number, ended: boolean, own: string | null, direction: Direction, equipment: Named,
  fuel: { sent: bigint | null; sentMissing: string | null; lost: bigint | null; lostMissing: string | null }): AgreementRow[] {
  const missingOf = (field: keyof EndedLeaseTerms) => (field in terms ? (terms as EndedLeaseTerms)[field] as string | undefined : undefined);
  const since = startHourOf(terms.activeSince, missingOf('activeSinceMissing'));
  const last = deliveryHourOf(terms.lastDelivery, missingOf('lastDeliveryMissing'));
  const giverMissing = missingOf('giverMissing') ?? null;
  const receiverMissing = missingOf('receiverMissing') ?? null;
  const partner = direction === 'sent' ? terms.receiver : terms.giver;
  const partnerMissing = direction === 'sent' ? receiverMissing : giverMissing;
  const base = { terms, at, ended, from: own, partner, partnerMissing, giver: terms.giver, giverMissing, receiver: terms.receiver, receiverMissing,
    since: since.value, sinceMissing: since.missing, last: last.value, lastMissing: last.missing };
  const schedule = new Map<string, string[]>();
  const add = (items: readonly MarketItem[] | null, text: (raw: bigint) => string) => {
    for (const item of items ?? []) schedule.set(item.variant, [...schedule.get(item.variant) ?? [], text(BigInt(item.amountRaw))]);
  };
  add(terms.monthly, raw => `Monthly ${units(raw)}`);
  add(terms.percentage, raw => `Production share ${share(raw)}`);
  add(terms.once, raw => `Once ${units(raw)}`);
  const poolMissing = (label: string, items: readonly MarketItem[] | null, missing: string | undefined, fallback: string) =>
    items === null ? (missing ? `${label}: ${missing}` : fallback) : null;
  const scheduleMissing = joinMissing([
    poolMissing('Monthly', terms.monthly, missingOf('monthlyMissing'), 'Monthly schedule not recorded'),
    poolMissing('Production share', terms.percentage, missingOf('percentageMissing'), 'Production share not recorded'),
    poolMissing('Once', terms.once, missingOf('onceMissing'), 'One-time schedule not recorded'),
  ]);
  const delivered = byVariant(terms.delivered), lost = byVariant(terms.sunk);
  const deliveredMissing = terms.delivered === null ? missingOf('deliveredMissing') ?? 'Delivered equipment not recorded' : null;
  const lostMissing = terms.sunk === null ? missingOf('sunkMissing') ?? 'Lost equipment not recorded' : null;
  const variants = [...new Set([...schedule.keys(), ...delivered.keys(), ...lost.keys()])];
  const out: AgreementRow[] = [];
  if (variants.length) out.push({ ...base, key: terms.key, fuel: false, items: variants.map(variant => {
    const drawn = equipment(variant);
    return { variant, name: drawn.name, icon: drawn.icon, missing: drawn.missing, schedule: (schedule.get(variant) ?? []).join(' · '),
      scheduleMissing, delivered: delivered.get(variant) ?? null, deliveredMissing: delivered.has(variant) ? null : deliveredMissing,
      lost: lost.get(variant) ?? null, lostMissing: lost.has(variant) ? null : lostMissing };
  }) });
  else if (scheduleMissing || deliveredMissing || lostMissing) out.push({ ...base, key: terms.key, fuel: false, items: [{
    variant: null, name: null, icon: null, missing: joinMissing([scheduleMissing, deliveredMissing, lostMissing]),
    schedule: '', scheduleMissing, delivered: null, deliveredMissing, lost: null, lostMissing }] });
  const daily = amountBigInt(terms.fuelDailyRaw, missingOf('fuelDailyRawMissing'));
  const percentage = amountBigInt(terms.fuelPercentageRaw, missingOf('fuelPercentageRawMissing'));
  const lastFuel = amountBigInt(terms.lastFuelDeliveredRaw, missingOf('lastFuelDeliveredRawMissing'));
  const fuelScheduleMissing = joinMissing([
    daily.missing ? `Daily: ${daily.missing}` : null,
    percentage.missing ? `Production share: ${percentage.missing}` : null,
  ]);
  if ((daily.value !== null && daily.value !== 0n) || (percentage.value !== null && percentage.value !== 0n)
    || (ended && ((lastFuel.value !== null && lastFuel.value !== 0n) || lastFuel.missing !== null))
    || (fuel.sent !== null && fuel.sent !== 0n) || (fuel.lost !== null && fuel.lost !== 0n)
    || fuelScheduleMissing !== null || fuel.sentMissing !== null || fuel.lostMissing !== null) {
    out.push({ ...base, key: `${terms.key}:fuel`, fuel: true, items: [{ variant: null, name: 'Fuel', icon: null, missing: null,
      schedule: [daily.value ? `Daily ${fuelText(daily.value)}` : '', percentage.value ? `Production share ${share(percentage.value)}` : ''].filter(Boolean).join(' · '),
      scheduleMissing: fuelScheduleMissing, delivered: fuel.sent, deliveredMissing: fuel.sentMissing, lost: fuel.lost, lostMissing: fuel.lostMissing }] });
  }
  return out;
}

export function agreementRows(reply: MarketAt, history: LendLeaseHistoryReply | undefined, direction: Direction,
  matches: (tag: string, hour: number) => boolean, showEnded: boolean, unrecorded: (hour: number) => string | null = () => null):
  { rows: AgreementRow[]; unattributed: number; membership: string | null } {
  const rows: AgreementRow[] = [];
  let unattributed = 0, membership: string | null = null;
  const take = (terms: AgreementTerms, at: number, ended: boolean, equipment: Named,
    fuel: { sent: bigint | null; sentMissing: string | null; lost: bigint | null; lostMissing: string | null }) => {
    const own = direction === 'sent' ? terms.giver : terms.receiver;
    if (own === null) { if (!ended) unattributed++; return; }
    if (matches(own, at)) rows.push(...rowsOf(terms, at, ended, own, direction, equipment, fuel));
    else membership ??= unrecorded(at);
  };
  const live = equipmentOf(reply);
  const standing = new Set<string>();
  for (const terms of reply.agreements ?? []) {
    standing.add(terms.key);
    take(terms, reply.hour, false, live, { sent: null, sentMissing: null, lost: null, lostMissing: null });
  }
  if (showEnded && history) {
    const named = equipmentOf(history);
    for (const lease of history.agreements) {
      if (lease.lastHour >= reply.hour || standing.has(lease.terms.key)) continue;
      const sent = amountBigInt(lease.fuelSentRaw, lease.fuelSentRawMissing), lost = amountBigInt(lease.fuelSunkRaw, lease.fuelSunkRawMissing);
      take(lease.terms, lease.lastHour, true, named, { sent: sent.value, sentMissing: sent.missing, lost: lost.value, lostMissing: lost.missing });
    }
  }
  return { rows, unattributed, membership };
}

export const amountOf = (row: Pick<AgreementRow, 'fuel'>, raw: bigint | null) => raw === null ? '—' : row.fuel ? fuelText(raw) : units(raw);

export function agreementValues(rows: readonly AgreementRow[], live: Live<LiveValues<number | bigint>>): Live<LiveValues<number | bigint>> {
  const byKey = new Map(rows.map(row => [row.key, row]));
  let made: { from: LiveValues<number | bigint>; value: LiveValues<number | bigint> } | null = null;
  const read = (): LiveValues<number | bigint> => {
    const from = live.read();
    if (made && made.from === from) return made.value;
    const split = (key: string) => { const at = key.lastIndexOf('\u0000'); return { row: byKey.get(key.slice(0, at)), column: key.slice(at + 1) }; };
    const readExact = (key: string): number | bigint | undefined => {
      const { row, column } = split(key);
      if (!row) return undefined;
      if (row.fuel && !row.ended) {
        const fuelKey = liveKey(row.terms.key, column === 'delivered' ? 'fuelSent' : 'fuelLost');
        return from.raw?.(fuelKey) ?? from.get(fuelKey);
      }
      const raw = row.items[0]?.[column === 'delivered' ? 'delivered' : 'lost'] ?? null;
      return raw === null ? undefined : raw;
    };
    const value: LiveValues<number | bigint> = {
      get: readExact,
      raw: key => {
        const held = readExact(key);
        return held === undefined ? undefined : typeof held === 'bigint' ? held : BigInt(Math.trunc(held));
      },
      missing: key => {
        const { row, column } = split(key);
        if (row?.fuel && !row.ended) return from.missing(liveKey(row.terms.key, column === 'delivered' ? 'fuelSent' : 'fuelLost'));
        const item = row?.items[0];
        return (column === 'delivered' ? item?.deliveredMissing : item?.lostMissing) ?? 'Not in this agreement';
      },
    };
    made = { from, value };
    return value;
  };
  return { read, subscribe: live.subscribe };
}


export type { EndedLease };
