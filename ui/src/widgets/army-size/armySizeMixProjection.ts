import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { projectForceAtMix, type ForceAtMixOptions } from '../../families/bars/forceAtMix.ts';
import { validateForceAtReply } from '../../families/bars/forceAtRace.ts';
import type { BarsModel, BarsProjection } from '../../families/bars/model.ts';
import { registerBarsProjection } from '../../families/bars/registry.ts';
import { buildShareRowsModel } from '../../families/bars/shareRows.ts';
import { nextSeriesRaceChange, seriesRaceUnavailable, seriesRaceWarnings } from '../../families/bars/seriesRace.ts';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourcePlan } from '../../families/contract.ts';
import type { ArmyAttribution } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';

export interface ArmySizeMixOptions extends ForceAtMixOptions {
  readonly countFor: ArmyAttribution;
}

const armyCountFor = (value: unknown): ArmyAttribution => (value === 'host' ? 'host' : 'owner');

export const armySizeMixProjection: BarsProjection<ArmySizeMixOptions> = registerBarsProjection({
  kind: 'army-size',
  family: 'bars',
  nextChange(readView, options) {
    return nextSeriesRaceChange(readView.hour, { ...options, candidates: [] });
  },
  project(readView: BoundedReadView, options: ArmySizeMixOptions): BarsModel {
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
    return projectForceAtMix(readView, validateForceAtReply(panel), options, {
      decimals: 0,
      valueFormat: { decimals: 0 },
      valueLabel: 'Battalions',
      partsLabel: 'Battalions by unit group',
      emptyMessage: 'No battalion mix recorded at this date.',
    });
  },
}, 'mix');

function armySizeMixSources(ctx: FamilyPlanContext): SourcePlan {
  const countFor = armyCountFor(ctx.options.countFor);
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
    countFor,
  };
  return {
    owner: `army-size:mix:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${countFor}`,
    entries: top == null && !all && !countries.length ? [] : [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage,
        product: 'army-at',
        followQuery: { panel: 'army-at', ...params },
        params,
        validate: validateForceAtReply,
      },
    }],
    allowEmptyFollow: top == null && !all && !countries.length,
  };
}

export const armySizeMixDeclaration: BarsWidgetDeclaration<ArmySizeMixOptions> = {
  family: 'bars',
  kind: 'army-size',
  label: 'Army composition',
  subject: 'armies',
  emptyCampaign: 'Load a campaign to see its army sizes.',
  icon: 'shield',
  optionFields: OPTIONS,
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    everyOnTop: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  sources: armySizeMixSources,
  projection: armySizeMixProjection,
  projectionOptions: ctx => {
    const ranked = ctx.countries.top != null || ctx.countries.all;
    return {
      countFor: armyCountFor(ctx.options.countFor),
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
