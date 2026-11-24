import { formatPower } from '../../dashboard/powerDecimals.ts';
import {
  categoryLabel,
  groupAt,
  powerGain,
  spendingGapNote,
  spendingGaps,
  spendingGroups,
  type PowerLedger,
  type SpendingSelection,
} from '../../dashboard/powerSpending.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { BreakdownModel, BreakdownProjection } from '../../families/breakdown/model.ts';
import { registerBreakdownProjection } from '../../families/breakdown/registry.ts';
import { buildBreakdownTree, type RawBreakdownLeaf } from '../../families/breakdown/tree.ts';

export interface PowerSpendingPanelData {
  readonly ledgers: readonly PowerLedger[];
  readonly from: number | null;
  readonly firstHour: number;
  readonly selection?: SpendingSelection;
}

export interface PowerSpendingProjectionOptions {
  readonly digits: number;
  readonly levels: readonly boolean[];
}

function shortGapLabel(hours: number): string {
  if (hours < 48) return `${hours} ${hours === 1 ? 'hour' : 'hours'} not recorded`;
  const days = Math.floor(hours / 24);
  return `${days.toLocaleString('en-US')} ${days === 1 ? 'day' : 'days'} not recorded`;
}

export const ppSpendingProjection: BreakdownProjection<PowerSpendingProjectionOptions> = {
  kind: 'political-power-spending-breakdown',
  project(readView: BoundedReadView, options: PowerSpendingProjectionOptions): BreakdownModel {
    const panel = readView.panel<PowerSpendingPanelData>('spending');
    const hour = Math.floor(readView.hour);
    if (!panel || !panel.ledgers.length) {
      return {
        hour,
        rootLabel: 'Spent',
        total: { known: false, value: null, text: '0', unit: 'PP', verb: 'Spent' },
        parts: [],
        empty: 'No political power spent in this period.',
      };
    }

    const { ledgers, from, firstHour, selection } = panel;
    if (ledgers.every(ledger => !ledger.coverage.length)) {
      return {
        hour,
        rootLabel: 'Spent',
        total: { known: false, value: null, text: 'Not recorded', unit: 'PP', verb: 'Spent' },
        parts: [],
        empty: 'Political power spending was not recorded.',
        emptyKind: 'unavailable',
      };
    }

    const groups = spendingGroups(ledgers, from, selection);
    const leaves: RawBreakdownLeaf[] = [];
    let anyUnknownCost = false;
    let anyUncovered = false;

    for (const group of groups.values()) {
      const now = groupAt(group, hour);
      if (!now) continue;
      const partial = now.unknown > 0 || now.uncovered;
      if (now.unknown > 0) anyUnknownCost = true;
      if (now.uncovered) anyUncovered = true;
      const catName = categoryLabel(group.category.name, group.category.identified);
      const categoryNode = {
        key: group.category.id,
        name: catName,
        neutral: !group.category.identified,
      };
      const path = group.latest.name === null
        ? [categoryNode]
        : [categoryNode, {
            key: group.id,
            name: group.latest.name,
            subtitle: `${catName} · ${formatDateOnly(now.latestHour)}`,
            note: group.latest.unknown ?? undefined,
            detailRows: group.latest.targetTag
              ? [{ label: 'Target', value: group.latest.targetName ?? group.latest.targetTag }]
              : undefined,
          }];
      leaves.push({
        path,
        value: Number(now.charge),
        exact: now.charge,
        count: now.payments,
        countLabel: 'Payments',
        partial,
        partialReason: partial ? 'Some payment costs were not recorded.' : undefined,
      });
    }

    const gaps = spendingGaps(ledgers, { from, hour }, selection);
    const gapNote = spendingGapNote(gaps);
    const gapHours = gaps.reduce((sum, gap) => sum + (gap.through - gap.from + 1), 0);
    const startHour = from ?? firstHour;
    const periodLabel = `Spent since ${formatDateOnly(startHour)}`;

    const gain = powerGain(ledgers, { from, hour }, selection);
    const comparison = gain
      ? {
          label: 'Gained',
          value: `${formatPower(gain.gained, options.digits)} PP`,
          ...(gain.lost > 0n ? { note: `${formatPower(gain.lost, options.digits)} PP lost at cap.` } : {}),
        }
      : undefined;

    if (!leaves.length) {
      return {
        hour,
        rootLabel: 'Spent',
        periodLabel,
        comparison,
        total: { known: true, value: 0, text: '0', unit: 'PP', verb: 'Spent' },
        parts: [],
        empty: 'No political power spent in this period.',
        emptyDetail: gapNote ?? undefined,
      };
    }

    const { parts, total } = buildBreakdownTree({
      leaves,
      levels: options.levels,
      formatValue: (value, exact) => formatPower(exact ?? BigInt(Math.round(value)), options.digits),
      unit: 'PP',
      verb: 'Spent',
      partialReason: gapNote ?? (anyUnknownCost ? 'Some payment costs were not recorded.' : anyUncovered ? 'Some payments may be missing.' : undefined),
      shortReason: gapHours > 0 ? shortGapLabel(gapHours) : (anyUnknownCost ? 'Some costs not recorded' : undefined),
    });

    return {
      hour,
      rootLabel: 'Spent',
      periodLabel,
      comparison,
      total,
      parts,
      warnings: [...new Set(ledgers.flatMap(l => l.warnings))],
    };
  },
};

registerBreakdownProjection(ppSpendingProjection);
