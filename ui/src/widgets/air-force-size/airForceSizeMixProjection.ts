import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { projectForceAtMix, type ForceAtMixOptions } from '../../families/bars/forceAtMix.ts';
import { validateForceAtReply } from '../../families/bars/forceAtRace.ts';
import type { BarsModel, BarsProjection } from '../../families/bars/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { buildShareRowsModel } from '../../families/bars/shareRows.ts';
import { nextSeriesRaceChange, seriesRaceUnavailable, seriesRaceWarnings } from '../../families/bars/seriesRace.ts';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourcePlan } from '../../families/contract.ts';
import type { AirForceMeasureOption, AirForceOwnershipOption } from './airForceSizeRaceProjection.ts';

export interface AirForceSizeMixOptions extends ForceAtMixOptions {
  readonly measure: AirForceMeasureOption;
  readonly ownership: AirForceOwnershipOption;
}

const airMeasure = (value: unknown): AirForceMeasureOption => (value === 'wings' ? 'wings' : 'planes');
const airOwnership = (value: unknown): AirForceOwnershipOption => (value === 'effective' ? 'effective' : 'roster');

export const airForceSizeMixProjection: BarsProjection<AirForceSizeMixOptions> = registerFamilyProjection({
  kind: 'air-force-size',
  family: 'bars',
  nextChange(readView, options) {
    return nextSeriesRaceChange(readView.hour, options);
  },
  project(readView: BoundedReadView, options: AirForceSizeMixOptions): BarsModel {
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
    const valueLabel = options.measure === 'wings' ? 'Number of wings' : 'Number of planes';
    const emptyMessage = options.measure === 'wings' ? 'No air wings recorded at this date.' : 'No planes in wings recorded at this date.';
    const note = options.measure === 'planes' ? 'Planes in wings; stockpiled planes are not included.' : undefined;
    return projectForceAtMix(readView, validateForceAtReply(panel), options, {
      decimals: 0,
      valueFormat: { decimals: 0 },
      valueLabel,
      unit: options.measure === 'wings' ? 'Wings' : 'Planes',
      partsLabel: 'Aircraft types',
      emptyMessage,
      ...(note ? { note } : {}),
    });
  },
}, 'mix');

function airForceSizeMixSources(ctx: FamilyPlanContext): SourcePlan {
  const measure = airMeasure(ctx.options.measure);
  const ownership = airOwnership(ctx.options.ownership);
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
    owner: `air-force-size:mix:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${measure}:${ownership}`,
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

export const airForceSizeMixDeclaration: BarsWidgetDeclaration<AirForceSizeMixOptions> = {
  family: 'bars',
  kind: 'air-force-size',
  label: 'Air force composition',
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
  sources: airForceSizeMixSources,
  projection: airForceSizeMixProjection,
  projectionOptions: ctx => {
    const ranked = ctx.countries.top != null || ctx.countries.all;
    return {
      measure: airMeasure(ctx.options.measure),
      ownership: airOwnership(ctx.options.ownership),
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
