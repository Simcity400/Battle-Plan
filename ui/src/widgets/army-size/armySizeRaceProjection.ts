import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { projectForceAtReply, validateForceAtReply, type ForceAtRaceBaseOptions } from '../../families/bars/forceAtRace.ts';
import type { BarsModel, BarsProjection } from '../../families/bars/model.ts';
import { registerBarsProjection } from '../../families/bars/registry.ts';
import {
  nextSeriesRaceChange,
  projectSeriesRace,
  seriesRaceSources,
  seriesRaceUnavailable,
  seriesRaceWarnings,
  type SeriesRaceBaseOptions,
} from '../../families/bars/seriesRace.ts';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourcePlan } from '../../families/contract.ts';
import type { ArmyAttribution, ArmyMeasure } from '../../generated/dashboard';
import { armyMeasure, armyMetadataRequest, measureName, validateArmySizeReply, type ArmySizeReply } from './armySizeData.ts';

export interface ArmySizeRaceOptions extends SeriesRaceBaseOptions, ForceAtRaceBaseOptions {
  readonly measure: ArmyMeasure;
  readonly countFor: ArmyAttribution;
}

const armyCountFor = (value: unknown): ArmyAttribution => (value === 'host' ? 'host' : 'owner');

function seriesMeasure(measure: ArmyMeasure, countFor: ArmyAttribution): string {
  const prefix = countFor === 'host' ? 'host' : 'owner';
  switch (measure) {
    case 'manpower': return `${prefix}Manpower`;
    case 'divisions': return `${prefix}Divisions`;
    case 'strength': return `${prefix}Strength`;
    case 'battalions': return `${prefix}Battalions`;
  }
}

export const armySizeRaceProjection: BarsProjection<ArmySizeRaceOptions> = registerBarsProjection({
  kind: 'army-size',
  family: 'bars',
  nextChange(readView, options) {
    return nextSeriesRaceChange(readView.hour, options);
  },
  project(readView: BoundedReadView, options: ArmySizeRaceOptions): BarsModel {
    const isStrength = options.measure === 'strength';
    const scaleUnit = options.measure === 'manpower'
      ? 'Manpower'
      : options.measure === 'divisions'
        ? 'Divisions'
        : isStrength
          ? 'Strength (HP)'
          : 'Battalions';
    const valueLabel = measureName(options.measure);
    const valueFormat = isStrength ? { decimals: 0, maxDecimals: 2 } : { decimals: 0 };
    const scaleFormat = isStrength ? { decimals: 0, maxDecimals: 2 } : { decimals: 0 };
    const emptyMessage = 'No known army sizes at this date.';

    if (!options.split || options.measure !== 'battalions') {
      return projectSeriesRace(readView, options, {
        decimals: isStrength ? 5 : 0,
        valueFormat,
        scaleFormat,
        scaleUnit,
        valueLabel,
        emptyMessage,
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
        scaleFormat,
        rows: [],
        empty: 'Group membership is unavailable at this time.',
        emptyKind: 'unavailable',
      };
    }

    const refused = readView.refused('main');
    if (refused) {
      return { form: 'ordered', hour, animateOrder: true, showAxis: true, scaleUnit, scaleFormat, rows: [], empty: refused, emptyKind: 'unavailable' };
    }

    const panel = readView.panel('main');
    if (panel === undefined) {
      return { form: 'ordered', hour, animateOrder: true, showAxis: true, scaleUnit, scaleFormat, rows: [] };
    }

    return projectForceAtReply(readView, validateForceAtReply(panel), options, {
      decimals: 0,
      valueFormat: { decimals: 0 },
      scaleFormat: { decimals: 0 },
      scaleUnit,
      valueLabel,
      partsLabel: 'Battalions by unit group',
      emptyMessage,
    });
  },
}, 'race');

function armySizeRaceSources(ctx: FamilyPlanContext<ArmySizeReply>): SourcePlan {
  const measure = armyMeasure(ctx.options.measure, ctx.aux?.measures);
  const countFor = armyCountFor(ctx.options.countFor);
  const byType = measure === 'battalions' && ctx.options.byType === true;
  if (!byType) {
    return seriesRaceSources(ctx, {
      kind: 'army-size',
      family: { kind: 'army-totals', measure: seriesMeasure(measure, countFor) },
    });
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
    countFor,
  };
  return {
    owner: `army-size:race:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${countFor}:byType`,
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

export const armySizeRaceDeclaration: BarsWidgetDeclaration<ArmySizeRaceOptions, ArmySizeReply> = {
  family: 'bars',
  kind: 'army-size',
  label: 'Army size',
  subject: 'armies',
  emptyCampaign: 'Load a campaign to see its army sizes.',
  icon: 'shield',
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    everyOnTop: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  aux: {
    product: 'army-size',
    params: track => armyMetadataRequest(track)!.params,
    validate: validateArmySizeReply,
  },
  sources: armySizeRaceSources,
  projection: armySizeRaceProjection,
  projectionOptions: ctx => {
    const ranked = ctx.countries.top != null || ctx.countries.all;
    const measure = armyMeasure(ctx.options.measure, ctx.aux?.measures);
    return {
      measure,
      countFor: armyCountFor(ctx.options.countFor),
      split: measure === 'battalions' && ctx.options.byType === true,
      candidates: ranked ? ctx.countries.every : ctx.countries.tags,
      holders: ranked ? null : ctx.countries.holders,
      membership: ranked ? null : ctx.countries.membership,
      unobserved: ranked ? [] : ctx.countries.unobserved,
      top: ctx.countries.top,
      all: ctx.countries.all,
    };
  },
  unavailable: seriesRaceUnavailable,
  warnings: (ctx, followRefused) => [
    ...(ctx.aux?.warnings ?? []),
    ...seriesRaceWarnings(followRefused),
  ],
  jumpToDate: true,
  keyByBranch: true,
};
