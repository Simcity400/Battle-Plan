import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import {
  factionTotalsLinesOptions,
  factionTotalsLinesSources,
  projectFactionTotalsLines,
  type FactionTotalsLinesOptions,
} from '../../families/time-chart/factionTotalsLines.ts';
import { factionStrengthSource } from './factionStrengthData.ts';

export const factionStrengthLinesProjection: TimeChartProjection<FactionTotalsLinesOptions> = registerFamilyProjection({
  kind: 'faction-strength',
  family: 'time-chart',
  variant: 'line',
  joinMode: 'step',
  measureTitle: 'Faction strength',
  project(readView: BoundedReadView, options: FactionTotalsLinesOptions) {
    return projectFactionTotalsLines(readView, options);
  },
}, 'lines');

function factionStrengthLinesSources(ctx: FamilyPlanContext): SourcePlan {
  const source = factionStrengthSource(ctx.options);
  const plan = factionTotalsLinesSources('faction-strength:lines', ctx, source.kind, source.measure);
  return plan.entries.length ? plan : { ...plan, refused: ctx.countries.reason ?? 'No faction strength is available for this selection.' };
}

export const factionStrengthLinesDeclaration: TimeChartWidgetDeclaration<FactionTotalsLinesOptions> = {
  family: 'time-chart',
  kind: 'faction-strength',
  label: 'Faction strength',
  subject: 'faction strength',
  emptyCampaign: 'Load a campaign to see faction strength.',
  icon: 'shield',
  countries: {
    mode: 'multi',
    content: 'history',
    allowSlots: true,
    factions: true,
    requireGroupsWhen: () => true,
  },
  sources: factionStrengthLinesSources,
  projection: factionStrengthLinesProjection,
  projectionOptions(ctx, extra) {
    const source = factionStrengthSource(ctx.options);
    return factionTotalsLinesOptions(
      'faction-strength',
      ctx,
      { seriesKind: source.kind, measure: source.measure, measureLabel: source.label, ...(source.note ? { note: source.note } : {}) },
      extra,
    );
  },
  unavailable: ctx => {
    if (ctx.widget.countries.kind === 'factions') return null;
    return ctx.countries.reason;
  },
};
