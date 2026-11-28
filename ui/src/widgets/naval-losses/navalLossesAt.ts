import { forceSelection, validateNavalLossesAtReply } from '../../families/bars/forceAtRace.ts';
import type { FamilyPlanContext, SourceEntry, SourcePlan } from '../../families/contract.ts';
import { navalAttribution, navalKind, navalPeriod } from './navalLossesData.ts';

export const NAVAL_NOTE = 'Counted when the game lists a sinking; a country is named as it was when the sinking was read.';

export function navalLossesAtSources(ctx: FamilyPlanContext, view: 'race' | 'mix'): SourcePlan {
  const kind = navalKind(ctx.options);
  const attribution = navalAttribution(ctx.options);
  const period = navalPeriod(ctx.options);
  const selection = forceSelection(ctx);
  const params = { branchId: ctx.branch.id, ...selection.params, attribution, kind, period };
  const entry: SourceEntry = {
    name: 'main',
    source: ctx.track.source,
    branch: ctx.branch.id,
    offset: ctx.track.axisOffset,
    read: {
      shape: 'point',
      channel: 'panel',
      coverage: selection.coverage,
      product: 'naval-losses-at',
      exactProduct: 'naval-losses-at',
      followQuery: { panel: 'naval-losses-at', ...params },
      params,
      dynamicKeys: ['hour', 'countries'],
      validate: validateNavalLossesAtReply,
    },
  };
  return {
    owner: `naval-losses:${view}:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${kind}:${attribution}:${period}`,
    entries: selection.none ? [] : [entry],
    allowEmptyFollow: selection.none,
  };
}
