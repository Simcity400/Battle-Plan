import type { ButterflyItem } from '../../charts/butterfly';
import { formatAmount } from '../../charts/valueAxis.ts';
import type { HolderLooks } from '../../dashboard/holderLooks';
import type { WarContributionCountry, WarContributionGroup, WarObservation, WarTimeline, WarsReply } from '../../generated/dashboard';
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
  { key: 'casualties', label: 'Casualties contribution', unknown: 'unknownCasualties' },
  { key: 'lendLeaseSent', label: 'Lend-lease sent' },
  { key: 'lendLeaseReceived', label: 'Lend-lease received' },
  { key: 'participation', label: 'Total contribution' },
] as const satisfies readonly { key: keyof WarContributionCountry; label: string; unknown?: keyof WarContributionCountry }[];
export type CategoryKey = (typeof CATEGORIES)[number]['key'];

export interface MemberPart { tag: string; value: bigint; unknown: bigint | null }
export interface SideTotal { total: bigint; unknown: bigint | null; members: MemberPart[] }
export interface ContributionRow { key: CategoryKey; label: string; left: SideTotal; right: SideTotal }

const distinct = (tags: readonly string[]) => [...new Set(tags)];

export function contributionRows(group: WarContributionGroup): ContributionRow[] {
  const scores = new Map<string, WarContributionCountry>();
  for (const row of group.participants) if (!scores.has(row.tag)) scores.set(row.tag, row);
  const side = (tags: readonly string[], key: CategoryKey, unknown?: keyof WarContributionCountry): SideTotal => {
    const members = distinct(tags).flatMap(tag => {
      const row = scores.get(tag);
      return row ? [{ tag, value: BigInt(row[key] as string), unknown: unknown ? BigInt(row[unknown] as string) : null }] : [];
    });
    return { members, total: members.reduce((sum, member) => sum + member.value, 0n),
      unknown: unknown ? members.reduce((sum, member) => sum + (member.unknown ?? 0n), 0n) : null };
  };
  return CATEGORIES.map(category => ({ key: category.key, label: category.label,
    left: side(group.viewpointSide, category.key, 'unknown' in category ? category.unknown : undefined),
    right: side(group.opposingSide, category.key, 'unknown' in category ? category.unknown : undefined) }));
}

export const toNumber = (raw: bigint) => Number(raw) / UNIT;
export const sideAmount = (side: SideTotal) => side.total + (side.unknown ?? 0n);

const magnitude = (raw: bigint) => raw < 0n ? -raw : raw;
export const sideMagnitude = (side: SideTotal) =>
  side.members.reduce((sum, member) => sum + magnitude(member.value) + magnitude(member.unknown ?? 0n), 0n);

export function rowLengths(row: ContributionRow, show: 'share' | 'totals'): { left: number; right: number } {
  const left = toNumber(sideMagnitude(row.left)), right = toNumber(sideMagnitude(row.right));
  if (show === 'share') return { left: left > 0 ? 1 : 0, right: right > 0 ? 1 : 0 };
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

export function shareText(value: bigint, total: bigint): string {
  if (total === 0n) return '—';
  const magnitude = value < 0n ? -value : value, whole = total < 0n ? -total : total;
  const hundredths = (magnitude * 20000n + whole) / (whole * 2n);
  return `${(value < 0n) !== (total < 0n) && hundredths ? '-' : ''}${hundredths / 100n}.${String(hundredths % 100n).padStart(2, '0')}%`;
}

export const warName = (observation: WarObservation | null) => observation?.name.known ? observation.name.text : 'Name not recorded';

export type Show = 'share' | 'totals';
export type Looks = Pick<HolderLooks, 'colour' | 'name' | 'nameFailure'>;
export type ContributionItem = ButterflyItem & { row: ContributionRow };

const amount = (raw: bigint, digits = 2) => formatAmount(raw, SCALE, digits);
export const formatContribution = (raw: bigint, participation: boolean) => participation ? `${amount(raw * 100n, 5)}%` : amount(raw, 5);
const sideText = (side: SideTotal, participation: boolean) => !side.members.length ? 'Not recorded'
  : participation ? `${amount(side.total * 100n)}%` : sideMagnitude(side) === 0n ? 'None yet' : amount(sideAmount(side));

export function contributionItems(rows: readonly ContributionRow[], show: Show, looks: Pick<Looks, 'colour'>, muted: string): ContributionItem[] {
  return rows.map(row => {
    const lengths = rowLengths(row, show), participation = row.key === 'participation';
    const parts = (side: SideTotal) => side.members.flatMap(member => [
      { id: member.tag, value: Math.abs(toNumber(member.value)), color: looks.colour(member.tag), negative: member.value < 0n },
      ...(member.unknown != null && member.unknown !== 0n ? [{ id: `${member.tag}:unknown`, value: Math.abs(toNumber(member.unknown)), color: looks.colour(member.tag), hatched: true,
        negative: member.unknown < 0n }] : []),
    ]);
    return { id: row.key, label: row.label, color: muted, row, segment: 'war',
      left: lengths.left, right: lengths.right, leftParts: parts(row.left), rightParts: parts(row.right),
      leftText: sideText(row.left, participation), rightText: sideText(row.right, participation), ratioText: '' };
  });
}

export function contributionStructure(group: WarContributionGroup | null, available: boolean): string {
  return JSON.stringify(group ? [group.viewpointSide, group.opposingSide, group.participants.map(row => row.tag), group.reason] : [available]);
}
