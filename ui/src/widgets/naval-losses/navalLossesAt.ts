import type { GatedHour } from '../../dashboard/regionTime';
import { forceSelection, validateNavalLossesAtReply, type ForceSelection } from '../../families/bars/forceAtRace.ts';
import type { FamilyPlanContext, SourceEntry, SourcePlan } from '../../families/contract.ts';
import type { NavalAttribution, NavalKind, NavalLossesAtReply, NavalPeriod } from '../../generated/dashboard';
import { navalAttribution, navalKind, navalPeriod } from './navalLossesData.ts';

export const NAVAL_NOTE = 'Counted when the game lists a sinking; a country is named as it was when the sinking was read.';

interface NavalLossesAtBase {
  readonly branchId: string;
  readonly selection: ForceSelection['params'];
  readonly attribution: NavalAttribution;
  readonly kind: NavalKind;
  readonly period: NavalPeriod;
}

function navalLossesAtParams(base: Readonly<Record<string, unknown>>, hour: GatedHour): Readonly<Record<string, unknown>> {
  const cfg = base as unknown as NavalLossesAtBase;
  return { branchId: cfg.branchId, hour, ...cfg.selection, attribution: cfg.attribution, kind: cfg.kind, period: cfg.period };
}

export function navalLossesAtProof(data: unknown, hour: number): readonly [number, number] {
  const reply = data as NavalLossesAtReply;
  return reply.firstHour <= hour && hour <= reply.lastHour ? [reply.firstHour, reply.lastHour] : [hour, hour];
}

export function navalLossesAtSources(ctx: FamilyPlanContext, view: 'race' | 'mix'): SourcePlan {
  const kind = navalKind(ctx.options);
  const attribution = navalAttribution(ctx.options);
  const period = navalPeriod(ctx.options);
  const selection = forceSelection(ctx);
  const base: NavalLossesAtBase = { branchId: ctx.branch.id, selection: selection.params, attribution, kind, period };
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
      followQuery: null,
      params: base as unknown as Readonly<Record<string, unknown>>,
      buildExactParams: navalLossesAtParams,
      provenInterval: navalLossesAtProof,
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
