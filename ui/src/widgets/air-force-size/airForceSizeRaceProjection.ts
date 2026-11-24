import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { projectForceAtReply, validateForceAtReply, type ForceAtRaceBaseOptions } from '../../families/bars/forceAtRace.ts';
import type { BarsModel, BarsProjection } from '../../families/bars/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import {
  nextSeriesRaceChange,
  projectSeriesRace,
  seriesRaceSources,
  seriesRaceUnavailable,
  seriesRaceWarnings,
  type SeriesRaceBaseOptions,
} from '../../families/bars/seriesRace.ts';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourcePlan } from '../../families/contract.ts';

export type AirForceMeasureOption = 'planes' | 'wings';
export type AirForceOwnershipOption = 'roster' | 'effective';

export interface AirForceSizeRaceOptions extends SeriesRaceBaseOptions, ForceAtRaceBaseOptions {
  readonly measure: AirForceMeasureOption;
  readonly ownership: AirForceOwnershipOption;
}

const airMeasure = (value: unknown): AirForceMeasureOption => (value === 'wings' ? 'wings' : 'planes');
const airOwnership = (value: unknown): AirForceOwnershipOption => (value === 'effective' ? 'effective' : 'roster');

function totalsFamily(measure: AirForceMeasureOption, ownership: AirForceOwnershipOption) {
  return { kind: 'air-force-totals', measure: `${ownership}${measure === 'wings' ? 'Wings' : 'Planes'}` };
}

export const airForceSizeRaceProjection: BarsProjection<AirForceSizeRaceOptions> = registerFamilyProjection({
  kind: 'air-force-size',
  family: 'bars',
  nextChange(readView, options) {
    return nextSeriesRaceChange(readView.hour, options);
  },
  project(readView: BoundedReadView, options: AirForceSizeRaceOptions): BarsModel {
    const scaleUnit = options.measure === 'wings' ? 'Air wings' : 'Planes';
    const valueLabel = options.measure === 'wings' ? 'Number of wings' : 'Number of planes';
    const emptyMessage = options.measure === 'wings' ? 'No air wings recorded at this date.' : 'No planes in wings recorded at this date.';
    const note = options.measure === 'planes' ? 'Planes in wings; stockpiled planes are not included.' : undefined;

    if (!options.split) {
      return projectSeriesRace(readView, options, {
        decimals: 0,
        valueFormat: { decimals: 0 },
        scaleFormat: { decimals: 0 },
        scaleUnit,
        valueLabel,
        emptyMessage,
        ...(note ? { note } : {}),
      });
    }

    const hour = readView.hour;
    const ranked = options.top != null || options.all === true;
    if (!ranked && options.unobserved?.some(([first, last]) => first <= hour && hour <= last)) {
      return {
        form: 'ordered',
        hour,
        animateOrder: true,
        showAxis: true,
        scaleUnit,
        scaleFormat: { decimals: 0 },
        rows: [],
        empty: 'Group membership is unavailable at this time.',
        emptyKind: 'unavailable',
      };
    }

    const refused = readView.refused('main');
    if (refused) {
      return { form: 'ordered', hour, animateOrder: true, showAxis: true, scaleUnit, scaleFormat: { decimals: 0 }, rows: [], empty: refused, emptyKind: 'unavailable' };
    }

    const panel = readView.panel('main');
    if (panel === undefined) {
      return { form: 'ordered', hour, animateOrder: true, showAxis: true, scaleUnit, scaleFormat: { decimals: 0 }, rows: [] };
    }

    return projectForceAtReply(readView, validateForceAtReply(panel), options, {
      decimals: 0,
      valueFormat: { decimals: 0 },
      scaleFormat: { decimals: 0 },
      scaleUnit,
      valueLabel,
      partsLabel: 'Aircraft types',
      emptyMessage,
      ...(note ? { note } : {}),
    });
  },
}, 'race');

function airForceSizeRaceSources(ctx: FamilyPlanContext): SourcePlan {
  const measure = airMeasure(ctx.options.measure);
  const ownership = airOwnership(ctx.options.ownership);
  const split = ctx.options.split === true;
  if (!split) {
    return seriesRaceSources(ctx, { kind: 'air-force-size', family: totalsFamily(measure, ownership) });
  }
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
    measure,
    ownership,
  };
  return {
    owner: `air-force-size:race:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${measure}:${ownership}:split`,
    entries: top == null && !all && !countries.length ? [] : [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage,
        product: 'air-force-at',
        followQuery: { panel: 'air-force-at', ...params },
        params,
        validate: validateForceAtReply,
      },
    }],
    allowEmptyFollow: top == null && !all && !countries.length,
  };
}

export const airForceSizeRaceDeclaration: BarsWidgetDeclaration<AirForceSizeRaceOptions> = {
  family: 'bars',
  kind: 'air-force-size',
  label: 'Air force size',
  subject: 'air forces',
  emptyCampaign: 'Load a campaign to see its air forces.',
  icon: 'layers',
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    everyOnTop: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  sources: airForceSizeRaceSources,
  projection: airForceSizeRaceProjection,
  projectionOptions: ctx => {
    const ranked = ctx.countries.top != null || ctx.countries.all;
    return {
      measure: airMeasure(ctx.options.measure),
      ownership: airOwnership(ctx.options.ownership),
      split: ctx.options.split === true,
      candidates: ranked ? ctx.countries.every : ctx.countries.tags,
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
