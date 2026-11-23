import { liveKey } from '../../charts/live.ts';
import type { Live, LiveValues } from '../../charts/live';
import { equipmentOf, FIXED5, FUEL_QUANTITY, fixedText, SHARE } from '../../dashboard/market.ts';
import type { MarketAt } from '../../dashboard/market';
import type { EndedLease, LeaseTerms, LendLeaseHistoryReply, MarketItem } from '../../generated/dashboard';

export type Direction = 'sent' | 'received';

export interface AgreementItem { variant: string | null; name: string | null; icon: string | null; missing: string | null; schedule: string;
  delivered: bigint | null; lost: bigint | null }

export interface AgreementRow {
  key: string;
  terms: LeaseTerms;
  at: number;
  ended: boolean;
  from: string | null;
  partner: string | null;
  giver: string | null;
  receiver: string | null;
  fuel: boolean;
  items: AgreementItem[];
  since: number | null;
  last: number | null;
}

const units = (raw: bigint) => fixedText(raw, FIXED5);
const share = (raw: bigint) => `${fixedText(raw, SHARE)}%`;
const fuelText = (raw: bigint) => fixedText(raw, FUEL_QUANTITY);
const byVariant = (list: readonly MarketItem[]) => new Map(list.map(item => [item.variant, BigInt(item.amountRaw)]));
const day = (hour: number) => hour > 0 ? hour : null;

type Named = ReturnType<typeof equipmentOf>;

function rowsOf(terms: LeaseTerms, at: number, ended: boolean, own: string | null, direction: Direction, equipment: Named,
  fuel: { sent: bigint | null; lost: bigint | null }): AgreementRow[] {
  const base = { terms, at, ended, from: own, partner: direction === 'sent' ? terms.receiver : terms.giver, giver: terms.giver, receiver: terms.receiver,
    since: day(terms.activeSince), last: day(terms.lastDelivery) };
  const schedule = new Map<string, string[]>();
  const add = (list: readonly MarketItem[], text: (raw: bigint) => string) => {
    for (const item of list) schedule.set(item.variant, [...schedule.get(item.variant) ?? [], text(BigInt(item.amountRaw))]);
  };
  add(terms.monthly, raw => `Monthly ${units(raw)}`);
  add(terms.percentage, raw => `Production share ${share(raw)}`);
  add(terms.once, raw => `Once ${units(raw)}`);
  const delivered = byVariant(terms.delivered), lost = byVariant(terms.sunk);
  const variants = [...new Set([...schedule.keys(), ...delivered.keys(), ...lost.keys()])];
  const out: AgreementRow[] = [];
  if (variants.length) out.push({ ...base, key: terms.key, fuel: false, items: variants.map(variant => {
    const drawn = equipment(variant);
    return { variant, name: drawn.name, icon: drawn.icon, missing: drawn.missing, schedule: (schedule.get(variant) ?? []).join(' · '),
      delivered: delivered.get(variant) ?? null, lost: lost.get(variant) ?? null };
  }) });
  const daily = BigInt(terms.fuelDailyRaw), percentage = BigInt(terms.fuelPercentageRaw);
  if (daily || percentage || fuel.sent || fuel.lost) out.push({ ...base, key: `${terms.key}:fuel`, fuel: true, items: [{ variant: null, name: 'Fuel', icon: null, missing: null,
    schedule: [daily ? `Daily ${fuelText(daily)}` : '', percentage ? `Production share ${share(percentage)}` : ''].filter(Boolean).join(' · '),
    delivered: fuel.sent, lost: fuel.lost }] });
  return out;
}

export function agreementRows(reply: MarketAt, history: LendLeaseHistoryReply | undefined, direction: Direction,
  matches: (tag: string, hour: number) => boolean, showEnded: boolean, unrecorded: (hour: number) => string | null = () => null):
  { rows: AgreementRow[]; unattributed: number; membership: string | null } {
  const rows: AgreementRow[] = [];
  let unattributed = 0, membership: string | null = null;
  const take = (terms: LeaseTerms, at: number, ended: boolean, equipment: Named, fuel: { sent: bigint | null; lost: bigint | null }) => {
    const own = direction === 'sent' ? terms.giver : terms.receiver;
    if (own === null) { unattributed++; return; }
    if (matches(own, at)) rows.push(...rowsOf(terms, at, ended, own, direction, equipment, fuel));
    else membership ??= unrecorded(at);
  };
  const live = equipmentOf(reply);
  const standing = new Set<string>();
  for (const terms of reply.agreements ?? []) {
    standing.add(terms.key);
    take(terms, reply.hour, false, live, { sent: null, lost: null });
  }
  if (showEnded && history) {
    const named = equipmentOf(history);
    for (const lease of history.agreements) {
      if (lease.lastHour >= reply.hour || standing.has(lease.terms.key)) continue;
      take(lease.terms, lease.lastHour, true, named, { sent: BigInt(lease.fuelSentRaw), lost: BigInt(lease.fuelSunkRaw) });
    }
  }
  return { rows, unattributed, membership };
}

export const amountOf = (row: Pick<AgreementRow, 'fuel'>, raw: bigint | null) => raw === null ? '—' : row.fuel ? fuelText(raw) : units(raw);

export function agreementValues(rows: readonly AgreementRow[], live: Live<LiveValues<number>>): Live<LiveValues<number>> {
  const byKey = new Map(rows.map(row => [row.key, row]));
  let made: { from: LiveValues<number>; value: LiveValues<number> } | null = null;
  const read = (): LiveValues<number> => {
    const from = live.read();
    if (made && made.from === from) return made.value;
    const split = (key: string) => { const at = key.lastIndexOf('\u0000'); return { row: byKey.get(key.slice(0, at)), column: key.slice(at + 1) }; };
    const value: LiveValues<number> = {
      get: key => {
        const { row, column } = split(key);
        if (!row) return undefined;
        if (row.fuel && !row.ended) return from.get(liveKey(row.terms.key, column === 'delivered' ? 'fuelSent' : 'fuelLost'));
        const raw = row.items[0]?.[column === 'delivered' ? 'delivered' : 'lost'] ?? null;
        return raw === null ? undefined : Number(raw);
      },
      missing: key => {
        const { row, column } = split(key);
        return row?.fuel && !row.ended ? from.missing(liveKey(row.terms.key, column === 'delivered' ? 'fuelSent' : 'fuelLost')) : 'Not in this agreement';
      },
    };
    made = { from, value };
    return value;
  };
  return { read, subscribe: live.subscribe };
}


export type { EndedLease };
