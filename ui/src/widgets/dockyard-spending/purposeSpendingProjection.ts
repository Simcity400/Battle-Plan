import { UNRESOLVED_ARM, domainName, unresolvedTypeName, variantNameAt } from '../../dashboard/equipment.ts';
import type { Domain, ProductionPurpose, ProductionSpendingLeaf } from '../../generated/dashboard';
import type { BoundedReadView } from '../../families/contract.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { BreakdownModel, BreakdownProjection } from '../../families/breakdown/model.ts';
import { registerBreakdownProjection } from '../../families/breakdown/registry.ts';
import { buildBreakdownTree, type RawBreakdownLeaf } from '../../families/breakdown/tree.ts';
import { UNIT, amountIn, formatAppliedIc, noProductionAt, spendingRecorded, unknownLabel, type SpendingData } from '../production-spending/spendingData.ts';
import {
  PURPOSES as DOCKYARD_PURPOSES,
  TYPE_NOTE,
  domainLeaves,
  formatExactIc,
  repairAmount,
  repairsRead,
  repairsSince,
  unresolvedLeaf,
} from './dockyardData.ts';
import { PURPOSES as FACTORY_PURPOSES } from '../military-factory-spending/factoryData.ts';

export interface PurposeSpendingProjectionOptions {
  readonly mode: 'dockyard' | 'military';
  readonly domain: Domain;
  readonly from: number | null;
  readonly firstHour: number;
  readonly digits: number;
  readonly levels: readonly boolean[];
}

function buildPurposeLeaves(
  reply: SpendingData,
  leaves: readonly ProductionSpendingLeaf[],
  hour: number,
  from: number | null,
  purposes: readonly { readonly id: string; readonly label: string; readonly leaf: ProductionPurpose }[],
): { readonly items: RawBreakdownLeaf[]; readonly anyUnproven: boolean; readonly anyUnresolved: boolean } {
  const observations = reply.equipmentObservations;
  const items: RawBreakdownLeaf[] = [];
  let anyUnproven = false;
  let anyUnresolved = false;

  const resolvePurpose = (purpose: ProductionPurpose | null) => {
    if (purpose === 'unknown') return { key: 'purpose-unrecorded', name: 'Purpose not recorded', neutral: true };
    const matched = purposes.find(p => p.leaf === purpose);
    if (matched) return { key: matched.id, name: matched.label, neutral: false };
    return { key: 'other-purpose', name: 'Other purposes', neutral: true };
  };

  for (const leaf of leaves) {
    const amount = amountIn(leaf.runs, from, hour, observations);
    if (unresolvedLeaf(leaf)) {
      if (amount.exact !== 0n) anyUnresolved = true;
      continue;
    }
    const unproven = leaf.unproven.length > 0 && amountIn(leaf.unproven, from, hour, observations).exact !== 0n;
    if (unproven) anyUnproven = true;
    if (amount.exact === 0n && !unproven) continue;

    const p = resolvePurpose(leaf.purpose);
    items.push({
      path: [
        p,
        { key: leaf.typeKey, name: leaf.name ?? unresolvedTypeName(leaf.typeKey) },
        { key: leaf.variant, name: variantNameAt(leaf.names, hour, leaf.variant) },
      ],
      value: amount.exact == null ? null : Number(amount.exact),
      exact: amount.exact,
      notRecorded: amount.exact == null,
      missingReason: amount.reason ? unknownLabel(amount.reason) : undefined,
      partial: unproven,
      partialReason: unproven ? 'Whether some of this output was refits or conversions was not recorded.' : undefined,
      count: 1,
      countLabel: 'Designs',
    });
  }

  const equipTotal = amountIn(reply.equipmentTotal, from, hour, observations);
  if (equipTotal.exact == null) {
    for (const p of purposes) {
      items.push({
        path: [{ key: p.id, name: p.label }],
        value: null,
        exact: null,
        notRecorded: true,
        missingReason: unknownLabel(equipTotal.reason ?? 'outsideRead'),
      });
    }
  }

  if (anyUnresolved) {
    items.push({
      path: [{ key: UNRESOLVED_ARM, name: domainName(UNRESOLVED_ARM), neutral: true }],
      value: null,
      exact: null,
      notRecorded: true,
      missingReason: TYPE_NOTE,
    });
  }

  return { items, anyUnproven, anyUnresolved };
}

export const purposeSpendingProjection: BreakdownProjection<PurposeSpendingProjectionOptions> = {
  kind: 'purpose-spending',
  project(readView: BoundedReadView, options: PurposeSpendingProjectionOptions): BreakdownModel {
    const reply = readView.panel<SpendingData>('spending');
    const hour = Math.floor(readView.hour);
    const isDockyard = options.mode === 'dockyard';
    const rootLabel = isDockyard ? 'All dockyard IC' : 'All military factory IC';
    const measureName = isDockyard ? 'dockyard IC' : 'military factory IC';
    const periodLabel = `Used since ${formatDateOnly(options.from ?? options.firstHour)}`;

    if (!reply) {
      return {
        hour,
        rootLabel,
        periodLabel,
        total: { known: false, value: null, text: '0', unit: UNIT, verb: 'Used' },
        parts: [],
        empty: `No ${measureName} spent yet.`,
      };
    }

    const read = spendingRecorded(reply) || (isDockyard && repairsRead(reply, hour));
    if (!read) {
      return {
        hour,
        rootLabel,
        periodLabel,
        total: { known: false, value: null, text: 'Not recorded', unit: UNIT, verb: 'Used' },
        parts: [],
        empty: `${isDockyard ? 'Dockyard spending' : 'Military factory spending'} was not recorded.`,
        emptyKind: 'unavailable',
        emptyAction: { label: 'Show by month', option: 'view', value: 'months' },
      };
    }

    const purposes = isDockyard
      ? [
          { id: DOCKYARD_PURPOSES[0].id, label: DOCKYARD_PURPOSES[0].label, leaf: 'new' as const },
          { id: DOCKYARD_PURPOSES[1].id, label: DOCKYARD_PURPOSES[1].label, leaf: 'refit' as const },
        ]
      : [
          { id: FACTORY_PURPOSES[0].id, label: FACTORY_PURPOSES[0].label, leaf: 'new' as const },
          { id: FACTORY_PURPOSES[1].id, label: FACTORY_PURPOSES[1].label, leaf: 'converted' as const },
        ];

    const { items, anyUnproven } = buildPurposeLeaves(
      reply,
      domainLeaves(reply, options.domain),
      hour,
      options.from,
      purposes,
    );

    const leaves: RawBreakdownLeaf[] = [...items];
    let repairsMissingReason: string | undefined;

    if (isDockyard) {
      const repairs = reply.repairs;
      if (!repairs) {
        repairsMissingReason = 'Repairs not recorded';
        leaves.push({
          path: [{ key: DOCKYARD_PURPOSES[2].id, name: DOCKYARD_PURPOSES[2].label }],
          value: null,
          exact: null,
          notRecorded: true,
          missingReason: repairsMissingReason,
        });
      } else {
        const win = { from: options.from, hour, end: hour };
        const totalNow = repairAmount(reply, repairs.total, win, hour);
        const denom = BigInt(repairs.denominator);
        if (totalNow.exact == null) {
          repairsMissingReason = 'Repairs not recorded';
          leaves.push({
            path: [{ key: DOCKYARD_PURPOSES[2].id, name: DOCKYARD_PURPOSES[2].label }],
            value: null,
            exact: null,
            notRecorded: true,
            missingReason: totalNow.reason ? unknownLabel(totalNow.reason) : repairsMissingReason,
          });
        } else if (totalNow.exact > 0n) {
          const since = repairsSince(reply, hour);
          const note = options.from == null && since != null ? `Dockyard repairs since ${formatDateOnly(since)}.` : 'Dockyard output used for repairs.';
          const variantParts = repairs.parts.filter(p => p.variant != null);
          let knownVariantSum = 0n;
          let anyPartMissing = false;
          for (const part of variantParts) {
            const amt = repairAmount(reply, part.runs, win, hour);
            if (amt.exact == null) {
              anyPartMissing = true;
              continue;
            }
            if (amt.exact === 0n) continue;
            knownVariantSum += amt.exact;
            leaves.push({
              path: [
                { key: DOCKYARD_PURPOSES[2].id, name: DOCKYARD_PURPOSES[2].label, note },
                { key: part.typeKey, name: part.name ?? unresolvedTypeName(part.typeKey) },
                { key: part.variant!, name: variantNameAt(part.names, hour, part.variant!) },
              ],
              value: Number(amt.exact) / Number(denom),
              exact: amt.exact,
              exactDenominator: denom,
              note,
              count: 1,
              countLabel: 'Designs',
            });
          }
          const remainder = totalNow.exact - knownVariantSum;
          if (remainder > 0n) {
            leaves.push({
              path: [
                { key: DOCKYARD_PURPOSES[2].id, name: DOCKYARD_PURPOSES[2].label, note },
                { key: 'unattributed', name: 'Unattributed', neutral: true },
              ],
              value: Number(remainder) / Number(denom),
              exact: remainder,
              exactDenominator: denom,
              note: anyPartMissing ? `${note} Split by design is not known.` : note,
            });
          }
        }
      }
    }

    const otherEmpty = !isDockyard || (reply.repairs ? repairAmount(reply, reply.repairs.total, { from: options.from, hour, end: hour }, hour).exact === 0n : false);
    if (!leaves.length || (noProductionAt(reply, hour) && otherEmpty)) {
      return {
        hour,
        rootLabel,
        periodLabel,
        total: { known: true, value: 0, text: '0', unit: UNIT, verb: 'Used' },
        parts: [],
        empty: `No ${measureName} spent yet.`,
      };
    }

    const { parts, total } = buildBreakdownTree({
      leaves,
      levels: options.levels,
      formatValue: (value, exact, denom) => exact == null
        ? formatAppliedIc(value, null, reply.decimals, options.digits)
        : formatExactIc(exact, denom, reply.decimals, options.digits),
      unit: UNIT,
      verb: 'Used',
      partialReason: repairsMissingReason ?? (anyUnproven ? 'Whether some output was refits or conversions was not recorded.' : undefined),
      shortReason: repairsMissingReason ?? (anyUnproven ? 'Some purposes not recorded' : undefined),
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

export const dockyardSpendingProjection: BreakdownProjection<PurposeSpendingProjectionOptions> = {
  ...purposeSpendingProjection,
  kind: 'dockyard-spending',
};

export const militaryFactorySpendingProjection: BreakdownProjection<PurposeSpendingProjectionOptions> = {
  ...purposeSpendingProjection,
  kind: 'military-factory-spending',
};

registerBreakdownProjection(purposeSpendingProjection);
registerBreakdownProjection(dockyardSpendingProjection);
registerBreakdownProjection(militaryFactorySpendingProjection);
