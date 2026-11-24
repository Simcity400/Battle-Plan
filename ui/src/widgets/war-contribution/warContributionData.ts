import type { ButterflyItem } from '../../charts/butterfly';
import { formatAmount } from '../../charts/valueAxis.ts';
import type { HolderLooks } from '../../dashboard/holderLooks';
import type { WarContributionGroup, WarMeasureKey, WarMeasureSide, WarObservation, WarTimeline, WarsReply } from '../../generated/dashboard';
import { labelObservationAt, observationAt } from '../wars/warsData.ts';

export const SCALE = 5;
const UNIT = 100000;

export const CATEGORIES = [
  { key: 'equipmentDamage', label: 'Equipment damage' },
  { key: 'occupation', label: 'Occupation' },
  { key: 'airDamage', label: 'Air damage' },
  { key: 'strategicBombing', label: 'Strategic bombing' },
  { key: 'shipDamage', label: 'Ship damage' },
  { key: 'convoyDamage', label: 'Convoy damage' },
  { key: 'casualties', label: 'Casualties contribution' },
  { key: 'lendLeaseSent', label: 'Lend-lease sent' },
  { key: 'lendLeaseReceived', label: 'Lend-lease received' },
  { key: 'participation', label: 'Total contribution' },
] as const satisfies readonly { key: WarMeasureKey; label: string }[];
export type CategoryKey = (typeof CATEGORIES)[number]['key'];

export interface ContributionRow { key: CategoryKey; label: string; left: WarMeasureSide; right: WarMeasureSide }

export function contributionRows(group: WarContributionGroup): ContributionRow[] {
  return CATEGORIES.flatMap(category => {
    const measure = group.measures.find(item => item.key === category.key);
    return measure ? [{ key: category.key, label: category.label, left: measure.viewpoint, right: measure.opposing }] : [];
  });
}

export const toNumber = (raw: bigint) => Number(raw) / UNIT;

export function rowLengths(row: ContributionRow, show: 'share' | 'totals'): { left: number; right: number } {
  const left = row.left.shareOfWar, right = row.right.shareOfWar;
  if (show === 'share') return { left: row.left.members.some(member => member.value !== '0') ? 1 : 0, right: row.right.members.some(member => member.value !== '0') ? 1 : 0 };
  if (left === null || right === null) return { left: 0, right: 0 };
  const larger = Math.max(left, right);
  return larger > 0 ? { left: left / larger, right: right / larger } : { left: 0, right: 0 };
}

export function chosenWar(reply: WarsReply, saved: string | null, at: number): WarTimeline | null {
  if (saved) {
    const war = reply.wars.find(item => item.id === saved);
    if (war) return war;
  }
  let best: { war: WarTimeline; active: boolean; count: number } | null = null;
  for (const war of reply.wars) {
    if (war.firstHour > at) continue;
    const active = observationAt(war, at) != null;
    const observation = labelObservationAt(war, Math.min(at, war.lastHour));
    const count = observation ? new Set([...observation.viewpointSide, ...observation.opposingSide]).size : 0;
    const better = !best || (active !== best.active ? active : count !== best.count ? count > best.count
      : war.lastHour !== best.war.lastHour ? war.lastHour > best.war.lastHour : Number(war.id) < Number(best.war.id));
    if (better) best = { war, active, count };
  }
  return best?.war ?? null;
}

export const readHourFor = (war: WarTimeline, at: number) => Math.min(Math.floor(at), war.lastHour);

export const shareText = (permyriad: number | null) => permyriad === null ? '\u2014' : `${Math.floor(permyriad / 100)}.${String(permyriad % 100).padStart(2, '0')}%`;

export const warName = (observation: WarObservation | null) => observation?.name.known ? observation.name.text : 'Name not recorded';

export type Show = 'share' | 'totals';
export type Looks = Pick<HolderLooks, 'colour' | 'name' | 'nameFailure'>;
export type ContributionItem = ButterflyItem & { row: ContributionRow };

const amount = (raw: bigint, digits = 2) => formatAmount(raw, SCALE, digits);
export const formatContribution = (raw: bigint, participation: boolean) => participation ? `${amount(raw * 100n, 5)}%` : amount(raw, 5);
const sideText = (side: WarMeasureSide, participation: boolean) => !side.members.length ? 'Not recorded'
  : participation ? `${amount(BigInt(side.total) * 100n)}%` : side.members.every(member => member.value === '0') ? 'None yet' : amount(BigInt(side.total));

export function contributionItems(rows: readonly ContributionRow[], show: Show, looks: Pick<Looks, 'colour'>, muted: string): ContributionItem[] {
  return rows.map(row => {
    const lengths = rowLengths(row, show), participation = row.key === 'participation';
    const parts = (side: WarMeasureSide) => side.members.map(member => {
      const value = BigInt(member.value);
      return { id: member.tag, value: Math.abs(toNumber(value)), color: looks.colour(member.tag), negative: value < 0n, ...(member.partial ? { hatched: true } : {}) };
    });
    return { id: row.key, label: row.label, color: muted, row, segment: 'war',
      left: lengths.left, right: lengths.right, leftParts: parts(row.left), rightParts: parts(row.right),
      leftText: sideText(row.left, participation), rightText: sideText(row.right, participation), ratioText: '' };
  });
}

export function contributionStructure(group: WarContributionGroup | null, available: boolean): string {
  return JSON.stringify(group ? [group.viewpointSide, group.opposingSide, group.participants.map(row => row.tag), group.reason] : [available]);
}
