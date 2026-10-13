import type { TooltipContent, TooltipRow } from '../../charts/tooltip';
import { usNumber } from '../../charts/numberFormat.ts';
import { formatDateTime } from '../../dashboard/calendar.ts';
import type { NameAt } from '../../dashboard/countryFacts';
import type { PeaceDeal } from '../../generated/dashboard';
import { transferHow, transferIsBy } from './territoryData.ts';
import type { CountryBalance, Counterpart, TransferOccurrence, TransferSide } from './territoryData';

export interface TransferStateAt { hour: number; state: number }
export interface TransferTipRequest { countries: NameAt[]; states: TransferStateAt[] }
type Label = { name: string; flag?: string; flagPending?: boolean; flagReason?: string };
export interface TransferLabels {
  country: (side: TransferSide) => Label;
  /** The country's dated name for a tag at an hour, such as an overlord. */
  tag: (tag: string, hour: number) => Label;
  state: (occurrence: Pick<TransferOccurrence, 'state' | 'afterHour'>) => string;
}

/** Names and flags a set of rows or one tooltip needs, each at its own read. */
export function transferRequest(occurrences: readonly TransferOccurrence[], balances: readonly CountryBalance[] = []): TransferTipRequest {
  const countries = new Map<string, NameAt>();
  const states = new Map<string, TransferStateAt>();
  const add = (at: { tag: string; hour: number }) => countries.set(JSON.stringify([at.hour, at.tag]), { hour: at.hour, tag: at.tag });
  for (const occurrence of occurrences) {
    add(occurrence.to);
    if (occurrence.from) add(occurrence.from);
    states.set(JSON.stringify([occurrence.afterHour, occurrence.state]), { hour: occurrence.afterHour, state: occurrence.state });
  }
  for (const balance of balances) {
    add(balance.side);
    for (const counterpart of [...balance.from.values(), ...balance.to.values()]) add(counterpart.side);
  }
  return { countries: [...countries.values()], states: [...states.values()] };
}

const states = (count: number) => `${usNumber.format(count)} ${count === 1 ? 'state' : 'states'}`;
const withVictoryPoints = (count: number, victoryPoints: bigint | null) =>
  victoryPoints == null ? states(count) : `${states(count)} · ${usNumber.format(victoryPoints)} VP`;
function counterparts(list: ReadonlyMap<string, Counterpart>, labels: TransferLabels): TooltipRow[] {
  return [...list.values()].sort((a, b) => b.count - a.count || labels.country(a.side).name.localeCompare(labels.country(b.side).name))
    .map(({ side, count }) => ({ label: labels.country(side).name, countryFlag: labels.country(side), value: usNumber.format(count), indent: true }));
}

/** Who a country gained from and lost to, one line per counterpart. */
export function countryTooltip(balance: CountryBalance, labels: TransferLabels): TooltipContent {
  const label = labels.country(balance.side);
  const rows: TooltipRow[] = [];
  if (balance.gained) {
    rows.push({ label: 'Gained', value: withVictoryPoints(balance.gained, balance.gainedVictoryPoints), strong: true }, ...counterparts(balance.from, labels));
    if (balance.unattributed) rows.push({ label: 'Former owner not recorded', value: usNumber.format(balance.unattributed), indent: true, tone: 'muted' });
  }
  if (balance.lost) rows.push({ label: 'Lost', value: withVictoryPoints(balance.lost, balance.lostVictoryPoints), strong: true, divider: rows.length > 0 },
    ...counterparts(balance.to, labels));
  return { title: label.name, countryFlag: label, rows, scrollable: true, scrollId: `country:${balance.key}` };
}

/** One state's transfer: date, war, how and victory points. A comparison of two
 * reads dates the change "by" its later read and says when it was still unchanged. */
export function transferTooltip(occurrence: TransferOccurrence, deal: PeaceDeal | undefined, labels: TransferLabels): TooltipContent {
  const giver = occurrence.from ? labels.country(occurrence.from).name : 'Former owner not recorded';
  const taker = labels.country(occurrence.to).name;
  const by = transferIsBy(occurrence);
  const how = transferHow(occurrence, deal);
  const subject = how.subjectOf == null ? null : `subject of ${labels.tag(how.subjectOf, occurrence.afterHour).name}`;
  const manner = [how.action, how.released ? 'Released' : null].filter(Boolean).join(' · ');
  const howText = manner ? subject ? `${manner}, ${subject}` : manner : subject ? subject[0].toUpperCase() + subject.slice(1) : null;
  const victoryPoints = occurrence.victoryPoints != null ? usNumber.format(occurrence.victoryPoints)
    : occurrence.victoryPointsPending ? 'Not read yet' : 'Not recorded';
  const rows: TooltipRow[] = [
    { label: 'Date', value: `${by ? 'By ' : ''}${formatDateTime(occurrence.afterHour)}` },
    deal?.warName != null ? { label: 'War', value: deal.warName } : { label: 'War', value: 'Not recorded', tone: 'muted' },
    ...(howText ? [{ label: 'How', value: howText }] : []),
    { label: 'Victory points', value: victoryPoints, ...(occurrence.victoryPoints == null ? { tone: 'muted' as const } : {}) },
  ];
  return { title: labels.state(occurrence), subtitle: `${giver} → ${taker}`, rows,
    description: by ? `It changed hands after ${formatDateTime(occurrence.beforeHour)}; the exact hour was not recorded.` : undefined };
}
