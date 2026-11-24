import { colour } from '../../charts/palette.ts';
import type { TooltipContent } from '../../charts/tooltip.ts';
import { formatAmount } from '../../charts/valueAxis.ts';
import type { WarContributionCountry, WarContributionGroup, WarObservation } from '../../generated/dashboard';
import { flagArtHandle } from '../art/provider.ts';
import type { BoundedReadView, FamilyTheme } from '../contract.ts';
import type { FamilyNavTuple } from '../host/keyboard.ts';
import type { ListboxMirrorItem } from '../host/mirror.ts';
import { buildTooltipContent, type TooltipListItem } from '../host/tooltips.ts';
import type { ArtResult } from '../plan.ts';
import type { AuditRow } from '../../draw/notifications';

export const WC_SCALE = 5;
export const WC_UNIT = 100000;

export const WC_MEASURES = [
  { key: 'participation', label: 'Participation', fullLabel: 'War participation', comparable: false },
  { key: 'equipmentDamage', label: 'Equipment', fullLabel: 'Equipment damage', comparable: true },
  { key: 'occupation', label: 'Territory', fullLabel: 'Territory occupied', comparable: true },
  { key: 'airDamage', label: 'Air', fullLabel: 'Air damage', comparable: true },
  { key: 'shipDamage', label: 'Naval', fullLabel: 'Naval damage', comparable: true },
  { key: 'casualties', label: 'Casualties', fullLabel: 'Casualties', comparable: true, unknownKey: 'unknownCasualties' },
  { key: 'strategicBombing', label: 'Bombing', fullLabel: 'Strategic bombing', comparable: true },
  { key: 'convoyDamage', label: 'Convoys', fullLabel: 'Convoy damage', comparable: true },
  { key: 'lendLeaseSent', label: 'Lend-lease out', fullLabel: 'Lend-lease sent', comparable: true },
  { key: 'lendLeaseReceived', label: 'Lend-lease in', fullLabel: 'Lend-lease received', comparable: true },
] as const;

export type WCMeasureKey = (typeof WC_MEASURES)[number]['key'];
export type WCViewMode = 'sides' | 'measure';
export type WCShowMode = 'share' | 'totals';

export interface WCMemberCell {
  readonly raw: bigint;
  readonly unknown: bigint;
  readonly shareOfSide: number | null;
  readonly shareOfWar: number | null;
  readonly displayText: string;
  readonly amountText: string;
  readonly isZero: boolean;
  readonly hasPartial: boolean;
}

export interface WCMemberRow {
  readonly key: string;
  readonly tag: string;
  readonly side: 'left' | 'right';
  readonly ordinal: number;
  readonly name: string;
  readonly color: string;
  readonly flagArt?: ArtResult;
  readonly cells: Readonly<Record<WCMeasureKey, WCMemberCell>>;
}

export interface WCSideSummary {
  readonly side: 'left' | 'right';
  readonly leadTag: string | null;
  readonly leadName: string;
  readonly leadColor: string;
  readonly leadFlagArt?: ArtResult;
  readonly members: readonly WCMemberRow[];
  readonly totals: Readonly<Record<WCMeasureKey, {
    readonly raw: bigint;
    readonly unknown: bigint;
    readonly warShare: number | null;
    readonly warShareText: string;
    readonly totalText: string;
    readonly hasPartial: boolean;
  }>>;
}

export interface WarContributionModel {
  readonly hour: number;
  readonly warTitle: string;
  readonly show: WCShowMode;
  readonly left: WCSideSummary;
  readonly right: WCSideSummary;
  readonly empty?: string;
  readonly emptyKind?: 'empty' | 'unavailable';
  readonly refused?: string;
  readonly warnings?: readonly string[];
}

export function formatWCPercent(fraction: number | null): string {
  if (fraction === null || !Number.isFinite(fraction)) return '—';
  const pct = fraction * 100;
  if (pct === 0) return '0%';
  if (Math.abs(pct) < 0.05) return '<0.1%';
  return `${pct.toFixed(1)}%`;
}

export function formatWCAmount(raw: bigint, participation: boolean): string {
  if (participation) {
    return `${formatAmount(raw * 100n, WC_SCALE, 1)}%`;
  }
  if (raw === 0n) return '0';
  return formatAmount(raw, WC_SCALE, 1);
}

export function projectWarContribution(
  readView: BoundedReadView,
  group: WarContributionGroup | null,
  observation: WarObservation | null,
  show: WCShowMode,
  themeSurface: string,
  warnings?: readonly string[],
): WarContributionModel {
  const hour = Math.floor(readView.hour);
  const warTitle = observation?.name.known ? observation.name.text : '';

  const emptySide = (side: 'left' | 'right'): WCSideSummary => {
    const totals = {} as Record<WCMeasureKey, WCSideSummary['totals'][WCMeasureKey]>;
    for (const m of WC_MEASURES) {
      totals[m.key] = { raw: 0n, unknown: 0n, warShare: null, warShareText: '—', totalText: '0', hasPartial: false };
    }
    return {
      side,
      leadTag: null,
      leadName: side === 'left' ? 'Viewpoint’s side' : 'Opposing side',
      leadColor: colour(themeSurface, side === 'left' ? 0 : 1),
      members: [],
      totals,
    };
  };

  if (!group || group.reason) {
    return {
      hour,
      warTitle,
      show,
      left: emptySide('left'),
      right: emptySide('right'),
      empty: group?.reason ?? 'This war’s contributions were not recorded at this time.',
      emptyKind: 'unavailable',
      warnings,
    };
  }

  const byTag = new Map<string, WarContributionCountry>();
  for (const row of group.participants) {
    if (!byTag.has(row.tag)) byTag.set(row.tag, row);
  }

  const leftTags = [...new Set(group.viewpointSide)];
  const rightTags = [...new Set(group.opposingSide)];

  const rawOf = (tag: string, key: WCMeasureKey): bigint => {
    const row = byTag.get(tag);
    return row ? BigInt(row[key] as string) : 0n;
  };
  const unknownOf = (tag: string, key: WCMeasureKey): bigint => {
    if (key !== 'casualties') return 0n;
    const row = byTag.get(tag);
    return row ? BigInt(row.unknownCasualties) : 0n;
  };

  const sideSums = (tags: readonly string[]) => {
    const sums: Record<WCMeasureKey, { raw: bigint; unknown: bigint }> = {} as never;
    for (const m of WC_MEASURES) {
      let raw = 0n;
      let unknown = 0n;
      for (const tag of tags) {
        raw += rawOf(tag, m.key);
        unknown += unknownOf(tag, m.key);
      }
      sums[m.key] = { raw, unknown };
    }
    return sums;
  };

  const leftSums = sideSums(leftTags);
  const rightSums = sideSums(rightTags);

  const warTotals: Record<WCMeasureKey, bigint> = {} as never;
  for (const m of WC_MEASURES) {
    const l = leftSums[m.key].raw + leftSums[m.key].unknown;
    const r = rightSums[m.key].raw + rightSums[m.key].unknown;
    warTotals[m.key] = (l < 0n ? -l : l) + (r < 0n ? -r : r);
  }

  const buildSide = (
    side: 'left' | 'right',
    tags: readonly string[],
    sums: Record<WCMeasureKey, { raw: bigint; unknown: bigint }>,
    colorOffset: number,
  ): WCSideSummary => {
    const members: WCMemberRow[] = tags.map((tag, ordinal) => {
      const who = readView.who('country', tag, hour);
      const name = who.name.status === 'ready' ? who.name.value : tag;
      const color = who.colour?.status === 'ready' ? who.colour.value : colour(themeSurface, colorOffset + ordinal);
      const flagArt = who.flagChoice?.status === 'ready' ? readView.art(flagArtHandle(who.flagChoice.value, 'panel')) : undefined;

      const cells = {} as Record<WCMeasureKey, WCMemberCell>;
      for (const m of WC_MEASURES) {
        const raw = rawOf(tag, m.key);
        const unknown = unknownOf(tag, m.key);
        const totalAmt = raw + unknown;
        const sideWhole = sums[m.key].raw + sums[m.key].unknown;
        const warWhole = warTotals[m.key];
        const shareOfSide = m.key === 'participation'
          ? Number(raw) / WC_UNIT
          : sideWhole !== 0n
            ? Number(totalAmt) / Number(sideWhole)
            : 0;
        const shareOfWar = m.key === 'participation'
          ? null
          : warWhole !== 0n
            ? Number(totalAmt) / Number(warWhole)
            : 0;
        const amountText = formatWCAmount(totalAmt, m.key === 'participation');
        const displayText = m.key === 'participation'
          ? formatWCPercent(Number(raw) / WC_UNIT)
          : show === 'totals'
            ? amountText
            : formatWCPercent(shareOfSide);
        cells[m.key] = {
          raw,
          unknown,
          shareOfSide,
          shareOfWar,
          displayText,
          amountText,
          isZero: raw === 0n && unknown === 0n,
          hasPartial: unknown !== 0n,
        };
      }

      return {
        key: `${side}:${tag}`,
        tag,
        side,
        ordinal,
        name,
        color,
        flagArt,
        cells,
      };
    });

    const totals = {} as Record<WCMeasureKey, WCSideSummary['totals'][WCMeasureKey]>;
    for (const m of WC_MEASURES) {
      const s = sums[m.key];
      const whole = s.raw + s.unknown;
      const warWhole = warTotals[m.key];
      const warShare = !m.comparable ? null : warWhole !== 0n ? Number(whole) / Number(warWhole) : 0;
      totals[m.key] = {
        raw: s.raw,
        unknown: s.unknown,
        warShare,
        warShareText: !m.comparable ? '—' : formatWCPercent(warShare),
        totalText: !m.comparable ? '—' : formatWCAmount(whole, false),
        hasPartial: s.unknown !== 0n,
      };
    }

    const lead = members[0];
    return {
      side,
      leadTag: lead?.tag ?? null,
      leadName: lead?.name ?? (side === 'left' ? 'Viewpoint’s side' : 'Opposing side'),
      leadColor: lead?.color ?? colour(themeSurface, colorOffset),
      leadFlagArt: lead?.flagArt,
      members,
      totals,
    };
  };

  const left = buildSide('left', leftTags, leftSums, 0);
  const right = buildSide('right', rightTags, rightSums, Math.max(1, leftTags.length));

  const title = warTitle || `${left.leadName} – ${right.leadName}`;

  return {
    hour,
    warTitle: title,
    show,
    left,
    right,
    warnings,
  };
}

export function sortSideMembers(members: readonly WCMemberRow[], sortKey: WCMeasureKey): WCMemberRow[] {
  return [...members].sort((a, b) => {
    const av = a.cells[sortKey].raw + a.cells[sortKey].unknown;
    const bv = b.cells[sortKey].raw + b.cells[sortKey].unknown;
    if (bv !== av) return bv > av ? 1 : -1;
    const ap = a.cells.participation.raw;
    const bp = b.cells.participation.raw;
    if (bp !== ap) return bp > ap ? 1 : -1;
    return a.ordinal - b.ordinal;
  });
}

export function buildWCTooltip(
  model: WarContributionModel,
  hitKey: string,
  activeMeasure: WCMeasureKey,
  _theme: FamilyTheme,
): TooltipContent | null {
  if (hitKey.startsWith('measure:')) {
    const key = hitKey.slice(8) as WCMeasureKey;
    const meta = WC_MEASURES.find(m => m.key === key);
    if (!meta) return null;
    const lt = model.left.totals[key];
    const rt = model.right.totals[key];
    return buildTooltipContent({
      title: meta.fullLabel,
      hour: model.hour,
      subtitle: model.warTitle,
      items: [
        {
          label: model.left.leadName,
          value: meta.comparable ? `${lt.warShareText} (${lt.totalText})` : '—',
          swatch: model.left.leadColor,
          strong: true,
        },
        {
          label: model.right.leadName,
          value: meta.comparable ? `${rt.warShareText} (${rt.totalText})` : '—',
          swatch: model.right.leadColor,
          strong: true,
        },
      ],
    });
  }

  const member = [...model.left.members, ...model.right.members].find(m => m.key === hitKey);
  if (!member) return null;
  const items: TooltipListItem[] = [];
  const activeMeta = WC_MEASURES.find(m => m.key === activeMeasure)!;
  const activeCell = member.cells[activeMeasure];
  items.push({
    label: activeMeta.fullLabel,
    value: activeMeasure === 'participation'
      ? activeCell.displayText
      : `${formatWCPercent(activeCell.shareOfSide)} (${activeCell.amountText})`,
    swatch: member.color,
    strong: true,
  });
  if (activeCell.hasPartial) {
    items.push({
      label: 'Unattributed casualties',
      value: formatWCAmount(activeCell.unknown, false),
      tone: 'muted',
      indent: true,
    });
  }
  for (const m of WC_MEASURES) {
    if (m.key === activeMeasure) continue;
    const c = member.cells[m.key];
    if (c.isZero) continue;
    items.push({
      label: m.label,
      value: m.key === 'participation' ? c.displayText : `${formatWCPercent(c.shareOfSide)} (${c.amountText})`,
    });
  }
  return buildTooltipContent({
    title: member.name,
    hour: model.hour,
    subtitle: member.side === 'left' ? model.left.leadName : model.right.leadName,
    items,
  });
}

export function deriveWCNavAndMirror(
  model: WarContributionModel,
  sortKey: WCMeasureKey,
): { readonly nav: readonly FamilyNavTuple[]; readonly mirror: readonly ListboxMirrorItem[] } {
  const nav: FamilyNavTuple[] = [];
  const mirror: ListboxMirrorItem[] = [];
  let idx = 0;
  for (const side of [model.left, model.right]) {
    for (const member of sortSideMembers(side.members, sortKey)) {
      const cell = member.cells[sortKey];
      nav.push([member.key, member.name, idx, -1, '', { tag: member.tag, side: member.side }]);
      mirror.push({
        key: member.key,
        spoken: `${member.name}, ${cell.displayText}`,
        index: idx,
      });
      idx++;
    }
  }
  return { nav, mirror };
}

export function deriveWCAudit(model: WarContributionModel, sortKey: WCMeasureKey): readonly AuditRow[] {
  const rows: AuditRow[] = [];
  for (const side of [model.left, model.right]) {
    for (const member of sortSideMembers(side.members, sortKey)) {
      const cell = member.cells[sortKey];
      rows.push({
        row: member.key,
        key: `${member.key}:${sortKey}`,
        value: cell.hasPartial ? `partial:${cell.amountText}` : cell.amountText,
      });
    }
  }
  return rows;
}
