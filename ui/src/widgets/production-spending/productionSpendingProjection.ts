import { UNRESOLVED_ARM, domainName, unresolvedTypeName, variantNameAt } from '../../dashboard/equipment.ts';
import type { BoundedReadView } from '../../families/contract.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { BreakdownModel, BreakdownProjection } from '../../families/breakdown/model.ts';
import { registerBreakdownProjection } from '../../families/breakdown/registry.ts';
import { buildBreakdownTree, type RawBreakdownLeaf } from '../../families/breakdown/tree.ts';
import type { FactoryKind } from './definition.ts';
import {
  UNIT,
  designAmount,
  formatAppliedIc,
  kindAmount,
  noProductionAt,
  spendingDesigns,
  spendingEmptyLabel,
  spendingRecorded,
  unknownLabel,
  type SpendingData,
} from './spendingData.ts';

export interface ProductionSpendingProjectionOptions {
  readonly kind: FactoryKind;
  readonly refits: boolean;
  readonly from: number | null;
  readonly firstHour: number;
  readonly digits: number;
  readonly levels: readonly boolean[];
}

export const productionSpendingProjection: BreakdownProjection<ProductionSpendingProjectionOptions> = {
  kind: 'production-spending',
  project(readView: BoundedReadView, options: ProductionSpendingProjectionOptions): BreakdownModel {
    const reply = readView.panel<SpendingData>('spending');
    const hour = Math.floor(readView.hour);
    const rootLabel = options.kind === 'dockyards' ? 'All naval production' : 'All military production';
    if (!reply) {
      return {
        hour,
        rootLabel,
        total: { known: false, value: null, text: '0', unit: UNIT, verb: 'Spent' },
        parts: [],
        empty: 'No military production yet.',
      };
    }
    if (!spendingRecorded(reply)) {
      return {
        hour,
        rootLabel,
        total: { known: false, value: null, text: 'Not recorded', unit: UNIT, verb: 'Spent' },
        parts: [],
        empty: 'Production spending was not recorded.',
        emptyKind: 'unavailable',
      };
    }

    const designs = spendingDesigns(reply, options.refits, options.kind);
    const totalAmt = kindAmount(reply, designs, options.from, hour, options.kind, options.refits);
    const periodLabel = `Spent since ${formatDateOnly(options.from ?? options.firstHour)}`;

    if (noProductionAt(reply, hour, options.refits) && totalAmt.exact === 0n) {
      return {
        hour,
        rootLabel,
        periodLabel,
        total: { known: true, value: 0, text: '0', unit: UNIT, verb: 'Spent' },
        parts: [],
        empty: 'No military production yet.',
      };
    }

    const leaves: RawBreakdownLeaf[] = [];
    for (const design of designs) {
      const amount = designAmount(reply, design, options.from, hour);
      if (amount.exact === 0n) continue;
      const identity = design.rocket ? `rocket:${design.variant}` : design.variant;
      const domainKey = design.rocket ? 'rocket' : (design.domain ?? UNRESOLVED_ARM);
      const domainLabel = design.rocket ? (reply.rocketName ?? reply.rocketKey ?? 'Rocket sites') : domainName(design.domain ?? UNRESOLVED_ARM);
      leaves.push({
        path: [
          { key: domainKey, name: domainLabel, neutral: !design.rocket && design.domain == null },
          { key: design.typeKey, name: design.name ?? unresolvedTypeName(design.typeKey) },
          { key: identity, name: variantNameAt(design.names, hour, design.variant) },
        ],
        value: amount.exact == null ? null : Number(amount.exact),
        exact: amount.exact,
        notRecorded: amount.exact == null,
        missingReason: amount.reason ? unknownLabel(amount.reason) : undefined,
        count: 1,
        countLabel: 'Designs',
      });
    }

    if (!leaves.length) {
      const empty = spendingEmptyLabel(reply, designs, hour, options.from, options.kind, options.refits);
      return {
        hour,
        rootLabel,
        periodLabel,
        total: { known: totalAmt.exact != null, value: totalAmt.exact == null ? null : Number(totalAmt.exact), text: '0', unit: UNIT, verb: 'Spent' },
        parts: [],
        empty,
        emptyKind: totalAmt.exact == null ? 'unavailable' : 'empty',
      };
    }

    const { parts, total } = buildBreakdownTree({
      leaves,
      levels: options.levels,
      formatValue: (value, exact) => formatAppliedIc(value, exact, reply.decimals, options.digits),
      unit: UNIT,
      verb: 'Spent',
      authoritativeTotal: totalAmt.exact != null
        ? { value: Number(totalAmt.exact), exact: totalAmt.exact }
        : { value: null, atLeast: true, reason: totalAmt.reason ? unknownLabel(totalAmt.reason) : 'Total not recorded' },
    });

    return {
      hour,
      rootLabel,
      periodLabel,
      total,
      parts,
      warnings: reply.warnings,
    };
  },
};

registerBreakdownProjection(productionSpendingProjection);
