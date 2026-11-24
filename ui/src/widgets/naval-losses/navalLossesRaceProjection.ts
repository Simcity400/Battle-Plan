import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { rowName, type FollowedAnswer } from '../../dashboard/follow/message.ts';
import type { GatedHour } from '../../dashboard/regionTime';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { projectForceAtReply, validateNavalLossesAtReply, type ForceAtRaceBaseOptions } from '../../families/bars/forceAtRace.ts';
import { countryRowPresentation, type BarsDataRow, type BarsModel, type BarsProjection } from '../../families/bars/model.ts';
import { registerBarsProjection } from '../../families/bars/registry.ts';
import {
  nextSeriesRaceChange,
  seriesRaceUnavailable,
  seriesRaceWarnings,
  type SeriesRaceBaseOptions,
} from '../../families/bars/seriesRace.ts';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourceEntry, SourcePlan } from '../../families/contract.ts';
import { holderTagAt, inHourRanges, type HourRanges } from '../../families/holders.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { valueFromNamedAnswer, valueFromWireCell, type Value } from '../../families/value.ts';
import { formatValueState } from '../../families/valueFormat.ts';
import type { NavalAttribution, NavalKind, NavalLossesAtReply, NavalPeriod } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import { navalAttribution, navalKind, navalMeasure, navalPeriod } from './navalLossesData.ts';

const NOTE = 'Counted when the game lists a sinking; a country is named as it was when the sinking was read.';

export interface NavalLossesRaceOptions extends SeriesRaceBaseOptions, ForceAtRaceBaseOptions {
  readonly kind: NavalKind;
  readonly attribution: NavalAttribution;
  readonly period: NavalPeriod;
}

interface NavalLossesExactBase {
  readonly branchId: string;
  readonly countries: readonly (string | number)[];
  readonly top: number | null;
  readonly attribution: NavalAttribution;
  readonly kind: NavalKind;
  readonly period: NavalPeriod;
  readonly tags: readonly string[];
  readonly membership: Readonly<Record<string, HourRanges>> | null;
}

function buildExactParams(base: Readonly<Record<string, unknown>>, hour: GatedHour): Readonly<Record<string, unknown>> {
  const cfg = base as unknown as NavalLossesExactBase;
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
  const activeTags = cfg.membership
    ? cfg.tags.filter(tag => inHourRanges(cfg.membership![tag], hour))
    : cfg.tags;
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
  const reply = data as NavalLossesAtReply | undefined;
  return reply && typeof reply.firstHour === 'number' && typeof reply.lastHour === 'number'
    ? [reply.firstHour, reply.lastHour]
    : null;
}

function numericOf(value: Value<bigint | number>): number | null {
  if (value.kind === 'known' || value.kind === 'partial') return Number(value.value);
  if (value.kind === 'zero') return 0;
  return null;
}

function makeFollowedRow(
  readView: BoundedReadView,
  key: string,
  displayTag: string,
  value: Value<bigint | number>,
  label: string,
): BarsDataRow {
  const pres = countryRowPresentation(readView, displayTag);
  const numeric = numericOf(value);
  const fmt = formatValueState(value, { decimals: 0 });
  const item: TooltipListItem = fmt.kind === 'notRecorded'
    ? { label, missingReason: fmt.reason }
    : fmt.kind === 'notApplicable'
      ? { label, value: '\u2014' }
      : { label, value: fmt.exact ?? fmt.text };
  return {
    kind: 'data',
    key,
    subject: displayTag,
    label: pres.label,
    labelMissing: pres.labelMissing,
    media: pres.media,
    ...(pres.color ? { color: pres.color } : {}),
    value: { value, format: { decimals: 0 }, label },
    numeric,
    tooltip: {
      title: pres.labelMissing ? 'Name not recorded' : pres.label,
      hour: readView.hour,
      items: [item],
      note: NOTE,
    },
  };
}

function compareExplicitRows(a: BarsDataRow, b: BarsDataRow): number {
  const aMissing = a.numeric === null || a.numeric === undefined;
  const bMissing = b.numeric === null || b.numeric === undefined;
  if (aMissing !== bMissing) return aMissing ? 1 : -1;
  if (!aMissing && !bMissing && a.numeric !== b.numeric) return b.numeric! - a.numeric!;
  return compareTags(a.subject ?? a.key, b.subject ?? b.key);
}

export const navalLossesRaceProjection: BarsProjection<NavalLossesRaceOptions> = registerBarsProjection({
  kind: 'naval-losses',
  family: 'bars',
  nextChange(readView, options) {
    return nextSeriesRaceChange(readView.hour, options);
  },
  project(readView: BoundedReadView, options: NavalLossesRaceOptions): BarsModel {
    const hour = readView.hour;
    const ranked = options.top != null || options.all === true;
    const label = navalMeasure({ kind: options.kind, attribution: options.attribution });
    const scaleUnit = options.kind === 'ships' ? 'Ships' : options.kind === 'convoys' ? 'Convoys' : 'Ships & convoys';
    const emptyMessage = options.period === 'month' ? 'No known naval losses this month.' : 'No known naval losses yet.';
    const baseModel = {
      form: 'ordered' as const,
      hour,
      animateOrder: true,
      showAxis: true,
      scaleUnit,
      scaleFormat: { decimals: 0 },
    };

    if (!ranked && options.unobserved?.some(([first, last]) => first <= hour && hour <= last)) {
      return { ...baseModel, rows: [], empty: 'Group membership is unavailable at this time.', emptyKind: 'unavailable' };
    }

    const refused = readView.refused('main');
    if (refused) {
      return { ...baseModel, rows: [], empty: refused, emptyKind: 'unavailable' };
    }

    const panel = readView.panel('main');
    if (panel !== undefined) {
      return projectForceAtReply(readView, validateNavalLossesAtReply(panel), options, {
        decimals: 0,
        valueFormat: { decimals: 0 },
        scaleFormat: { decimals: 0 },
        scaleUnit,
        valueLabel: label,
        partsLabel: 'Ships and convoys',
        emptyMessage,
        note: NOTE,
      });
    }

    const answer: FollowedAnswer | undefined = readView.answer('main');
    if (!answer) {
      return { ...baseModel, rows: [] };
    }
    if (answer.kind === 'notRecorded') {
      return { ...baseModel, rows: [], empty: answer.reason, emptyKind: 'unavailable' };
    }

    const rows: BarsDataRow[] = [];
    if (ranked) {
      for (let i = 0; i < answer.rows; i++) {
        const tag = rowName(answer, i);
        if (!tag) continue;
        const val = valueFromWireCell(answer, i, 0, { unrecordedReason: 'Not recorded at this hour' });
        const row = makeFollowedRow(readView, tag, tag, val, label);
        if (options.top != null && row.numeric !== null && row.numeric !== undefined && row.numeric <= 0) continue;
        rows.push(row);
      }
    } else {
      for (const candidate of options.candidates) {
        const datedTag = options.holders
          ? holderTagAt(options.holders[candidate], hour)
          : (isSlotKey(candidate) ? null : candidate);
        if (datedTag === null) continue;
        if (options.membership) {
          const inMember = inHourRanges(options.membership[candidate], hour) || inHourRanges(options.membership[datedTag], hour);
          if (!inMember) continue;
        }
        const val = valueFromNamedAnswer(answer, candidate, { unrecordedReason: 'Not recorded at this hour' });
        rows.push(makeFollowedRow(readView, candidate, datedTag, val, label));
      }
      rows.sort(compareExplicitRows);
    }

    return {
      ...baseModel,
      rows,
      ...(!rows.length ? { empty: emptyMessage, emptyKind: 'empty' as const } : {}),
    };
  },
}, 'race');

function navalLossesRaceSources(ctx: FamilyPlanContext): SourcePlan {
  const kind = navalKind(ctx.options);
  const attribution = navalAttribution(ctx.options);
  const period = navalPeriod(ctx.options);
  const split = ctx.options.split === true;
  const top = ctx.countries.top;
  const all = ctx.countries.all;
  const coverage: SourceCoverage = top != null
    ? { mode: 'population', form: 'top', limit: top }
    : all
      ? { mode: 'population', form: 'all' }
      : { mode: 'keyed', keys: ctx.countries.tags };
  const exactBase: NavalLossesExactBase = {
    branchId: ctx.branch.id,
    countries: top == null && !all ? ctx.countries.tags.map(countryFromKey) : [],
    top,
    attribution,
    kind,
    period,
    tags: ctx.countries.tags,
    membership: ctx.countries.membership,
  };
  const snapshotEntry: SourceEntry = {
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
  const owner = `naval-losses:race:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${kind}:${attribution}:${period}:${split ? 1 : 0}`;
  const canFollowWire = period === 'running' && !split && !all && (top != null || ctx.countries.tags.length > 0);
  if (!canFollowWire) {
    return {
      owner,
      entries: top == null && !all && !ctx.countries.tags.length ? [] : [snapshotEntry],
      staticEntry: snapshotEntry,
      exactEntry: snapshotEntry,
      allowEmptyFollow: top == null && !all && !ctx.countries.tags.length,
    };
  }
  const followParams = {
    branchId: ctx.branch.id,
    countries: top == null ? ctx.countries.tags.map(countryFromKey) : [],
    top,
    attribution,
    kind,
    period: 'running',
  };
  return {
    owner,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage,
        product: 'naval-losses',
        followQuery: { product: 'naval-losses', params: followParams },
        params: null,
      },
    }],
    staticEntry: snapshotEntry,
    exactEntry: snapshotEntry,
    allowEmptyFollow: false,
  };
}

export const navalLossesRaceDeclaration: BarsWidgetDeclaration<NavalLossesRaceOptions> = {
  family: 'bars',
  kind: 'naval-losses',
  label: 'Naval losses',
  subject: 'naval losses',
  emptyCampaign: 'Load a campaign to see its naval losses.',
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
  sources: navalLossesRaceSources,
  projection: navalLossesRaceProjection,
  projectionOptions: ctx => {
    const ranked = ctx.countries.top != null || ctx.countries.all;
    return {
      kind: navalKind(ctx.options),
      attribution: navalAttribution(ctx.options),
      period: navalPeriod(ctx.options),
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
