import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { projectForceAtMix, type ForceAtMixOptions } from '../../families/bars/forceAtMix.ts';
import { validateForceAtReply } from '../../families/bars/forceAtRace.ts';
import type { BarsModel, BarsProjection } from '../../families/bars/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { buildShareRowsModel } from '../../families/bars/shareRows.ts';
import { nextSeriesRaceChange, seriesRaceUnavailable, seriesRaceWarnings } from '../../families/bars/seriesRace.ts';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourcePlan } from '../../families/contract.ts';
import type { NavyGroupingOption, NavyMeasureOption, NavyOwnershipOption } from './navySizeRaceProjection.ts';

export interface NavySizeMixOptions extends ForceAtMixOptions {
  readonly measure: NavyMeasureOption;
  readonly ownership: NavyOwnershipOption;
  readonly grouping: NavyGroupingOption;
}

const navyMeasure = (value: unknown): NavyMeasureOption => (value === 'strength' ? 'strength' : 'ships');
const navyOwnership = (value: unknown): NavyOwnershipOption => (value === 'logical' ? 'logical' : 'roster');
const navyGrouping = (value: unknown): NavyGroupingOption => (value === 'type' ? 'type' : 'class');

export const navySizeMixProjection: BarsProjection<NavySizeMixOptions> = registerFamilyProjection({
  kind: 'navy-size',
  family: 'bars',
  nextChange(readView, options) {
    return nextSeriesRaceChange(readView.hour, options);
  },
  project(readView: BoundedReadView, options: NavySizeMixOptions): BarsModel {
    const hour = readView.hour;
    const ranked = options.top != null || options.all === true;
    if (!ranked && options.unobserved?.some(([first, last]) => first <= hour && hour <= last)) {
      return buildShareRowsModel({ hour, rows: [], empty: 'Group membership is unavailable at this time.', emptyKind: 'unavailable' });
    }
    const refused = readView.refused('main');
    if (refused) {
      return buildShareRowsModel({ hour, rows: [], empty: refused, emptyKind: 'unavailable' });
    }
    const panel = readView.panel('main');
    if (panel === undefined) {
      return buildShareRowsModel({ hour, rows: [] });
    }
    const strength = options.measure === 'strength';
    return projectForceAtMix(readView, validateForceAtReply(panel), options, {
      decimals: strength ? 5 : 0,
      valueFormat: strength ? { decimals: 0, maxDecimals: 2 } : { decimals: 0 },
      valueLabel: strength ? 'Current hull strength (HP)' : 'Number of ships',
      unit: strength ? 'Hull strength (HP)' : 'Ships',
      partsLabel: options.grouping === 'type' ? 'Roles' : 'Ship classes',
      emptyMessage: strength ? 'No hull strength recorded at this date.' : 'No ships recorded at this date.',
      ...(strength ? { referenceLabel: 'Maximum hull strength (HP)' } : {}),
    });
  },
}, 'mix');

function navySizeMixSources(ctx: FamilyPlanContext): SourcePlan {
  const measure = navyMeasure(ctx.options.measure);
  const ownership = navyOwnership(ctx.options.ownership);
  const grouping = navyGrouping(ctx.options.grouping);
  const top = ctx.countries.top;
  const all = ctx.countries.all;
  const countries = top == null && !all ? ctx.countries.tags.map(countryFromKey) : [];
  const coverage: SourceCoverage = top != null
    ? { mode: 'population', form: 'top', limit: top }
    : all
      ? { mode: 'population', form: 'all' }
      : { mode: 'keyed', keys: ctx.countries.tags };
  const params = {
    branchId: ctx.branch.id,
    countries,
    ...(top != null ? { top } : {}),
    ownership,
    grouping,
    measure,
  };
  return {
    owner: `navy-size:mix:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${measure}:${ownership}:${grouping}`,
    entries: top == null && !all && !countries.length ? [] : [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage,
        product: 'navy-at',
        followQuery: { panel: 'navy-at', ...params },
        params,
        validate: validateForceAtReply,
      },
    }],
    allowEmptyFollow: top == null && !all && !countries.length,
  };
}

export const navySizeMixDeclaration: BarsWidgetDeclaration<NavySizeMixOptions> = {
  family: 'bars',
  kind: 'navy-size',
  label: 'Navy composition',
  subject: 'navies',
  emptyCampaign: 'Load a campaign to see its navies.',
  icon: 'flows',
  countries: {
    mode: 'multi',
    allowSlots: true,
    everyOnTop: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  sources: navySizeMixSources,
  projection: navySizeMixProjection,
  projectionOptions: ctx => {
    const ranked = ctx.countries.top != null || ctx.countries.all;
    return {
      measure: navyMeasure(ctx.options.measure),
      ownership: navyOwnership(ctx.options.ownership),
      grouping: navyGrouping(ctx.options.grouping),
      holders: ranked ? null : ctx.countries.holders,
      membership: ranked ? null : ctx.countries.membership,
      unobserved: ranked ? [] : ctx.countries.unobserved,
      top: ctx.countries.top,
      all: ctx.countries.all,
    };
  },
  unavailable: seriesRaceUnavailable,
  warnings: (_ctx, followRefused) => seriesRaceWarnings(followRefused),
  jumpToDate: true,
  keyByBranch: true,
};
