import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { GatedHour } from '../../dashboard/regionTime';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { projectForceAtMix, type ForceAtMixOptions } from '../../families/bars/forceAtMix.ts';
import { validateNavalLossesAtReply } from '../../families/bars/forceAtRace.ts';
import type { BarsModel, BarsProjection } from '../../families/bars/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { buildShareRowsModel } from '../../families/bars/shareRows.ts';
import { nextSeriesRaceChange, seriesRaceUnavailable, seriesRaceWarnings } from '../../families/bars/seriesRace.ts';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourceEntry, SourcePlan } from '../../families/contract.ts';
import { inHourRanges, type HourRanges } from '../../families/holders.ts';
import type { NavalAttribution, NavalKind, NavalLossesAtReply, NavalPeriod } from '../../generated/dashboard';
import { navalAttribution, navalKind, navalMeasure, navalPeriod } from './navalLossesData.ts';

const NOTE = 'Counted when the game lists a sinking; a country is named as it was when the sinking was read.';

export interface NavalLossesMixOptions extends ForceAtMixOptions {
  readonly kind: NavalKind;
  readonly attribution: NavalAttribution;
  readonly period: NavalPeriod;
}

interface NavalLossesMixExactBase {
  readonly branchId: string;
  readonly top: number | null;
  readonly attribution: NavalAttribution;
  readonly kind: NavalKind;
  readonly period: NavalPeriod;
  readonly tags: readonly string[];
  readonly membership: Readonly<Record<string, HourRanges>> | null;
}

function buildExactParams(base: Readonly<Record<string, unknown>>, hour: GatedHour): Readonly<Record<string, unknown>> {
  const cfg = base as unknown as NavalLossesMixExactBase;
  if (cfg.top != null) {
    return {
      branchId: cfg.branchId,
      hour,
      countries: [],
      top: cfg.top,
      attribution: cfg.attribution,
      kind: cfg.kind,
      period: cfg.period,
    };
  }
  const activeTags = cfg.membership ? cfg.tags.filter(tag => inHourRanges(cfg.membership![tag], hour)) : cfg.tags;
  return {
    branchId: cfg.branchId,
    hour,
    countries: activeTags.map(countryFromKey),
    attribution: cfg.attribution,
    kind: cfg.kind,
    period: cfg.period,
  };
}

function provenInterval(data: unknown): readonly [number, number] | null {
  const reply = data as Partial<NavalLossesAtReply> | null | undefined;
  return reply && typeof reply.firstHour === 'number' && typeof reply.lastHour === 'number' && reply.firstHour <= reply.lastHour
    ? [reply.firstHour, reply.lastHour]
    : null;
}

export const navalLossesMixProjection: BarsProjection<NavalLossesMixOptions> = registerFamilyProjection({
  kind: 'naval-losses',
  family: 'bars',
  nextChange(readView, options) {
    return nextSeriesRaceChange(readView.hour, options);
  },
  project(readView: BoundedReadView, options: NavalLossesMixOptions): BarsModel {
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
    const label = navalMeasure({ kind: options.kind, attribution: options.attribution });
    const emptyMessage = options.period === 'month' ? 'No known naval losses this month.' : 'No known naval losses yet.';
    return projectForceAtMix(readView, validateNavalLossesAtReply(panel), options, {
      decimals: 0,
      valueFormat: { decimals: 0 },
      valueLabel: label,
      partsLabel: 'Ships and convoys',
      emptyMessage,
      note: NOTE,
    });
  },
}, 'mix');

function navalLossesMixSources(ctx: FamilyPlanContext): SourcePlan {
  const kind = navalKind(ctx.options);
  const attribution = navalAttribution(ctx.options);
  const period = navalPeriod(ctx.options);
  const top = ctx.countries.top;
  const all = ctx.countries.all;
  const coverage: SourceCoverage = top != null
    ? { mode: 'population', form: 'top', limit: top }
    : all
      ? { mode: 'population', form: 'all' }
      : { mode: 'keyed', keys: ctx.countries.tags };
  const exactBase: NavalLossesMixExactBase = {
    branchId: ctx.branch.id,
    top,
    attribution,
    kind,
    period,
    tags: ctx.countries.tags,
    membership: ctx.countries.membership,
  };
  const entry: SourceEntry = {
    name: 'main',
    source: ctx.track.source,
    branch: ctx.branch.id,
    offset: ctx.track.axisOffset,
    read: {
      shape: 'point',
      channel: 'panel',
      coverage,
      product: 'naval-losses-at',
      exactProduct: 'naval-losses-at',
      followQuery: null,
      params: exactBase as unknown as Readonly<Record<string, unknown>>,
      buildExactParams,
      provenInterval,
      dynamicKeys: ['hour', 'countries'],
      validate: validateNavalLossesAtReply,
    },
  };
  const emptySelection = top == null && !all && !ctx.countries.tags.length;
  return {
    owner: `naval-losses:mix:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${kind}:${attribution}:${period}`,
    entries: emptySelection ? [] : [entry],
    staticEntry: entry,
    exactEntry: entry,
    allowEmptyFollow: emptySelection,
  };
}

export const navalLossesMixDeclaration: BarsWidgetDeclaration<NavalLossesMixOptions> = {
  family: 'bars',
  kind: 'naval-losses',
  label: 'Naval losses mix',
  subject: 'naval losses',
  emptyCampaign: 'Load a campaign to see its naval losses.',
  icon: 'shield',
  countries: {
    mode: 'multi',
    labels: true,
    colours: true,
    allowSlots: true,
    everyOnTop: true,
    requireGroupsWhen: widget => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
  },
  sources: navalLossesMixSources,
  projection: navalLossesMixProjection,
  projectionOptions: ctx => {
    const ranked = ctx.countries.top != null || ctx.countries.all;
    return {
      kind: navalKind(ctx.options),
      attribution: navalAttribution(ctx.options),
      period: navalPeriod(ctx.options),
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
