import type { AgreementHistory, MarketHistoryReply } from '../../generated/dashboard';
import type { DrillAction } from '../../charts/drill/drillState';
import { fixedText, numberAt, valueAt } from '../../dashboard/economyHistory.ts';
import { FIXED5, FUEL_QUANTITY, holderTagAt, poolAt, readHourOf, variantAt } from '../../dashboard/marketHistory.ts';

export type Direction = 'sent' | 'received';

/** One agreement (or its fuel) as the table shows it at an hour. */
export interface AgreementRow {
  key: string;
  agreement: AgreementHistory;
  /** The hour its values are read at: the shown hour, or its last read when ended. */
  at: number;
  ended: boolean;
  /** The chosen country's side (`from`) and the other side, by the direction asked for. */
  from: string | null;
  partner: string | null;
  giver: string | null;
  receiver: string | null;
  fuel: boolean;
  /** The schedule's items: variant (or null for fuel), name and written amounts. */
  items: { variant: string | null; name: string | null; schedule: string; delivered: bigint | null; lost: bigint | null }[];
  since: number | null;
  last: number | null;
}

const units = (raw: bigint) => fixedText(raw, FIXED5);
const share = (raw: bigint) => `${fixedText(raw, 1000n, 2)}%`;
const fuel = (raw: bigint) => fixedText(raw, FUEL_QUANTITY);

/** The game's three schedule forms of an item, as recorded at `at`. */
function schedules(agreement: AgreementHistory, at: number) {
  const byVariant = new Map<string, string[]>();
  const add = (variant: string, text: string) => byVariant.set(variant, [...byVariant.get(variant) ?? [], text]);
  for (const item of poolAt(agreement.items, 0, at)) add(item.variant, `Monthly ${units(item.amount)}`);
  for (const item of poolAt(agreement.items, 1, at)) add(item.variant, `Production share ${share(item.amount)}`);
  for (const item of poolAt(agreement.items, 2, at)) add(item.variant, `Once ${units(item.amount)}`);
  return byVariant;
}

/** Every agreement the chosen countries send (or receive) at `hour`, with ended ones when
 * `ended` holds; fuel terms get their own row. Matching follows each side's tag at the read. */
export function agreementRows(reply: MarketHistoryReply, direction: Direction, matches: (tag: string, hour: number) => boolean, hour: number,
  showEnded: boolean): AgreementRow[] {
  const rows: AgreementRow[] = [];
  for (const agreement of reply.agreements) {
    const read = readHourOf(agreement.receiver, hour);
    if (!read || read.ended && !showEnded) continue;
    const { at, ended } = read;
    const giver = holderTagAt(reply, agreement.giver, at);
    const receiver = holderTagAt(reply, valueAt(agreement.receiver, at), at);
    const own = direction === 'sent' ? giver : receiver;
    if (own === null || !matches(own, at)) continue;
    const base = { agreement, at, ended, from: own, partner: direction === 'sent' ? receiver : giver, giver, receiver,
      since: numberAt(agreement.since, at), last: numberAt(agreement.last, at) };
    const scheduled = schedules(agreement, at);
    const delivered = new Map(poolAt(agreement.items, 3, at).map(item => [item.variant, item.amount]));
    const lost = new Map(poolAt(agreement.items, 4, at).map(item => [item.variant, item.amount]));
    const variants = [...new Set([...scheduled.keys(), ...delivered.keys(), ...lost.keys()])];
    if (variants.length) rows.push({ ...base, key: `${agreement.giver}:${agreement.id}`, fuel: false,
      items: variants.map(variant => ({ variant, name: variantAt(reply, variant, at), schedule: (scheduled.get(variant) ?? []).join(' · '),
        delivered: delivered.get(variant) ?? null, lost: lost.get(variant) ?? null })) });
    const [daily, percentage, sent, sunk] = [0, 1, 2, 3].map(index => { const value = valueAt(agreement.fuel[index] ?? [], at); return value === null ? null : BigInt(value); });
    if (daily || percentage || sent || sunk) rows.push({ ...base, key: `${agreement.giver}:${agreement.id}:fuel`, fuel: true,
      items: [{ variant: null, name: 'Fuel', schedule: [daily ? `Daily ${fuel(daily)}` : '', percentage ? `Production share ${share(percentage)}` : ''].filter(Boolean).join(' · '),
        delivered: sent, lost: sunk }] });
  }
  return rows;
}

/** Fuel quantities are /32768, equipment fixed5. */
export const amountOf = (row: AgreementRow, raw: bigint | null) => raw === null ? '—' : row.fuel ? fuel(raw) : units(raw);

/** Delivered (or lost) of the row's first item, for sorting and bars. */
export const firstAmount = (row: AgreementRow, which: 'delivered' | 'lost') => row.items[0]?.[which] ?? null;

/** Opens a row's drill at its read: an ended agreement stays at its last read, as no
 * later hour's reply holds it, whatever the playhead or playback. */
export const openAgreement = (row: Pick<AgreementRow, 'at' | 'ended'>, breakdown: string, playing: boolean, playhead: number | null): DrillAction =>
  ({ type: 'open', breakdown, hour: row.at, playing, playhead: row.ended ? null : playhead });
