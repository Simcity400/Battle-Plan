import type { ProductRequest } from '../../dashboard/client';
import type { ParityList } from '../../dashboard/parityRows';
import type { SunburstSettings } from '../../charts/sunburst/sunburstOptions';
import type { LossesReply } from '../../generated/dashboard';
import { lossKey, sourceKey } from './lossesData.ts';
import type { LossItem, LossTotal, LossWindow } from './lossesData';

interface LossesParityState {
  campaignKey: string | undefined;
  request: ProductRequest | null;
  data: LossesReply | undefined;
  window: LossWindow | null;
  items: readonly LossItem[];
  total: LossTotal;
  levels: readonly boolean[];
  settings: SunburstSettings;
  complete: boolean;
}

/** The complete inputs already supplied to the Sunburst, before its visual fold.
 * Raw source points retain signed digits, supplying hours and range operands;
 * acquisition and recorded unknowns remain separate from capture readiness. */
export function lossesParity(state: LossesParityState): ParityList {
  const { request, data, items } = state;
  return {
    kind: 'chart', label: 'Equipment losses', complete: state.complete,
    source: request ? { id: request.sourceId, revision: request.revision } : null,
    selection: { campaignKey: state.campaignKey, params: request?.params ?? null },
    window: state.window,
    levels: state.levels,
    settings: state.settings,
    authoritativeTotal: { ...state.total, exact: state.total.exact?.toString() ?? null },
    content: data ? {
      causes: data.causes.map(({ leaves: _leaves, ...cause }) => cause),
      designs: data.designs, warnings: data.warnings,
    } : null,
    coverage: data ? {
      branchId: data.branchId, tag: data.tag, landSource: data.landSource,
      amountScale: data.amountScale, landLowerBound: data.landLowerBound,
      destructionCountedFrom: data.destructionCountedFrom, retainedMonths: data.retainedMonths,
      land: data.coverage, destruction: data.destructionCoverage,
    } : null,
    operands: data?.causes.filter(cause => cause.loss).map(cause => ({
      key: sourceKey(cause.source), source: cause.source, domain: cause.domain,
      unknown: cause.unknown, unknownSpans: cause.unknownSpans,
      leaves: cause.leaves.map(leaf => {
        const design = leaf.design === null ? undefined : data.designs[leaf.design];
        return { key: lossKey(cause, design), variant: design?.variant ?? null,
          lowerBoundFrom: leaf.lowerBoundFrom, points: leaf.points };
      }),
    })) ?? [],
    total: items.length,
    rows: items.map((item, index) => ({
      key: JSON.stringify(item.path.map(part => part.id)), index,
      item: { ...item, path: item.path.map(({ label: _label, ...part }) => part),
        exact: item.exact?.toString() ?? null },
    })),
  };
}
