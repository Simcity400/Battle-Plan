import { validateRankReply } from '../../charts/race/measureRaceData.ts';
import { countryFromKey, countryKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { rowName, type FollowedAnswer } from '../../dashboard/follow/message.ts';
import type { GatedHour } from '../../dashboard/regionTime';
import { compareTags } from '../../dashboard/tagOrder.ts';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourceEntry, SourcePlan } from '../contract.ts';
import { holderTagAt, nextHoldersChange, nextMembershipChange, nextRangesChange, type HolderRun, type HourRanges } from '../holders.ts';
import type { TooltipListItem } from '../host/tooltips.ts';
import { knownValue, valueFromNamedAnswer, valueFromRawBigInt, valueFromWireCell, zeroValue, type Value } from '../value.ts';
import { formatValueState, type ValueFormat } from '../valueFormat.ts';
import type { SeriesStateReply } from '../../generated/dashboard';
import { countryRowPresentation, type BarsDataRow, type BarsModel } from './model.ts';

const ALL_POPULATION_TOP = 10_000;

export interface SeriesRaceExactBase {
  readonly branchId: string;
  readonly family: { readonly kind: string; readonly measure: string };
  readonly top: number | null;
  readonly tags: readonly string[];
  readonly membership: Readonly<Record<string, HourRanges>> | null;
}

export interface SeriesRaceBaseOptions {
  readonly candidates: readonly string[];
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly unobserved?: HourRanges;
  readonly top: number | null;
  readonly all?: boolean;
}

export interface SeriesRaceConfig {
  readonly kind: string;
  readonly family: { readonly kind: string; readonly measure: string };
  readonly ownerTag?: string;
}

export interface SeriesRaceProjectConfig {
  readonly decimals: number;
  readonly valueFormat: ValueFormat;
  readonly scaleFormat: ValueFormat;
  readonly scaleUnit: string;
  readonly valueLabel: string;
  readonly emptyMessage: string;
  readonly note?: string;
  readonly positiveOnly?: boolean;
}

export function buildSeriesRaceExactParams(
  base: Readonly<Record<string, unknown>>,
  hour: GatedHour,
): Readonly<Record<string, unknown>> | null {
  const cfg = base as unknown as SeriesRaceExactBase;
  if (cfg.top != null) {
    return { branchId: cfg.branchId, family: cfg.family, hour, top: cfg.top };
  }
  const activeTags = cfg.membership
    ? cfg.tags.filter(tag => cfg.membership![tag]?.some(([first, last]) => first <= hour && hour <= last))
    : cfg.tags;
  return {
    branchId: cfg.branchId,
    family: cfg.family,
    reads: activeTags.map(tag => ({ country: countryFromKey(tag), hour })),
  };
}

export function seriesRaceSources(
  ctx: FamilyPlanContext,
  config: SeriesRaceConfig,
): SourcePlan {
  const effectiveTop = ctx.countries.top ?? (ctx.countries.all ? Math.max(ALL_POPULATION_TOP, ctx.countries.every.length) : null);
  const coverage: SourceCoverage = ctx.countries.top != null
    ? { mode: 'population', form: 'top', limit: ctx.countries.top }
    : ctx.countries.all
      ? { mode: 'population', form: 'all' }
      : { mode: 'keyed', keys: ctx.countries.tags };
  const owner = `${config.kind}:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${config.family.kind}:${config.family.measure}${config.ownerTag ? `:${config.ownerTag}` : ''}`;

  const exactBase: SeriesRaceExactBase = {
    branchId: ctx.branch.id,
    family: config.family,
    top: effectiveTop,
    tags: ctx.countries.tags,
    membership: ctx.countries.membership,
  };

  const exactEntry: SourceEntry = {
    name: 'main',
    source: ctx.track.source,
    branch: ctx.branch.id,
    offset: ctx.track.axisOffset,
    read: {
      shape: 'point',
      channel: 'followed',
      coverage,
      product: 'series-state',
      exactProduct: 'series-state',
      followQuery: null,
      params: exactBase as unknown as Readonly<Record<string, unknown>>,
      buildExactParams: buildSeriesRaceExactParams,
      dynamicKeys: ['hour', 'reads'],
      validate: validateRankReply,
    },
  };

  const entries: SourceEntry[] = [];
  if (effectiveTop != null) {
    entries.push({
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage,
        product: 'series-state',
        followQuery: { product: 'series-state', params: { branchId: ctx.branch.id, family: config.family, top: effectiveTop } },
        params: null,
      },
    });
  } else if (ctx.countries.tags.length) {
    entries.push({
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage,
        product: 'series-state',
        followQuery: {
          product: 'series-state',
          coverage: 'row',
          params: { branchId: ctx.branch.id, family: config.family, reads: ctx.countries.tags.map(tag => ({ country: countryFromKey(tag) })) },
        },
        params: null,
      },
    });
  }

  return {
    owner,
    entries,
    staticEntry: exactEntry,
    exactEntry,
    allowEmptyFollow: effectiveTop == null && entries.length === 0,
  };
}

export function nextSeriesRaceChange(hour: number, options: SeriesRaceBaseOptions): number {
  return Math.min(
    nextHoldersChange(options.holders, hour),
    nextRangesChange(options.unobserved, hour),
    nextMembershipChange(options.membership, hour),
  );
}

export function seriesRaceUnavailable(ctx: FamilyPlanContext): string | null {
  if (!ctx.countries.all && ctx.countries.top == null && ctx.following && !ctx.countries.tags.length && ctx.countries.membership === null) {
    return 'No countries available for this selection.';
  }
  return null;
}

export function seriesRaceWarnings(followRefused: string | null): readonly string[] {
  return followRefused ? [`Values are not followed here: ${followRefused}.`] : [];
}

function scaleValue(raw: Value<bigint | number>, decimals: number): { readonly value: Value<bigint | number>; readonly numeric: number | null } {
  if (raw.kind === 'notRecorded' || raw.kind === 'notApplicable') {
    return { value: raw, numeric: null };
  }
  if (raw.kind === 'zero') {
    return { value: zeroValue(), numeric: 0 };
  }
  const factor = decimals > 0 ? 10 ** decimals : 1;
  const num = Number(raw.value) / factor;
  if (decimals === 0) {
    return { value: raw.value === 0 || raw.value === 0n ? zeroValue() : knownValue(raw.value), numeric: num };
  }
  return { value: num === 0 ? zeroValue() : knownValue(num), numeric: num };
}

function makeRow(
  readView: BoundedReadView,
  key: string,
  displayTag: string,
  rawValue: Value<bigint | number>,
  config: SeriesRaceProjectConfig,
): BarsDataRow {
  const pres = countryRowPresentation(readView, displayTag);
  const { value, numeric } = scaleValue(rawValue, config.decimals);
  const formatted = formatValueState(value, config.valueFormat);
  const item: TooltipListItem = formatted.kind === 'notRecorded'
    ? { label: config.valueLabel, missingReason: formatted.reason }
    : formatted.kind === 'notApplicable'
      ? { label: config.valueLabel, value: '\u2014' }
      : { label: config.valueLabel, value: formatted.exact ?? formatted.text };
  return {
    kind: 'data',
    key,
    subject: displayTag,
    label: pres.label,
    labelMissing: pres.labelMissing,
    media: pres.media,
    ...(pres.color ? { color: pres.color } : {}),
    value: {
      value,
      format: config.valueFormat,
      label: config.valueLabel,
    },
    numeric,
    tooltip: {
      title: pres.labelMissing ? 'Name not recorded' : pres.label,
      hour: readView.hour,
      items: [item],
      ...(config.note ? { note: config.note } : {}),
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

export function projectSeriesRace(
  readView: BoundedReadView,
  options: SeriesRaceBaseOptions,
  config: SeriesRaceProjectConfig,
): BarsModel {
  const hour = readView.hour;
  const ranked = options.top != null || options.all === true;
  const baseModel = {
    form: 'ordered' as const,
    hour,
    animateOrder: true,
    showAxis: true,
    scaleUnit: config.scaleUnit,
    scaleFormat: config.scaleFormat,
  };

  if (!ranked && options.unobserved?.some(([first, last]) => first <= hour && hour <= last)) {
    return {
      ...baseModel,
      rows: [],
      empty: 'Group membership is unavailable at this time.',
      emptyKind: 'unavailable',
    };
  }

  const refused = readView.refused('main');
  if (refused) {
    return { ...baseModel, rows: [], empty: refused, emptyKind: 'unavailable' };
  }

  const panel = readView.panel('main');
  if (panel !== undefined) {
    const reply: SeriesStateReply = validateRankReply(panel);
    const rows: BarsDataRow[] = [];
    for (const state of reply.states) {
      const key = countryKey(state.country);
      const displayTag = options.holders ? (holderTagAt(options.holders[key], hour) ?? key) : key;
      if (!ranked && options.membership) {
        const inMember = options.membership[key]?.some(([first, last]) => first <= hour && hour <= last)
          || options.membership[displayTag]?.some(([first, last]) => first <= hour && hour <= last);
        if (!inMember) continue;
      }
      const value = valueFromRawBigInt(
        state.run?.valueRaw ?? null,
        { kind: 'notRecorded', reason: state.missing ?? 'Not recorded at this hour' },
      );
      const row = makeRow(readView, key, displayTag, value, config);
      if (ranked && config.positiveOnly && row.numeric !== null && row.numeric !== undefined && row.numeric <= 0) continue;
      rows.push(row);
    }
    if (!ranked) rows.sort(compareExplicitRows);
    return {
      ...baseModel,
      rows,
      ...(!rows.length ? { empty: config.emptyMessage, emptyKind: 'empty' as const } : {}),
    };
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
      const rawValue = valueFromWireCell(answer, i, 0, { unrecordedReason: 'Not recorded at this hour' });
      const row = makeRow(readView, tag, tag, rawValue, config);
      if (config.positiveOnly && row.numeric !== null && row.numeric !== undefined && row.numeric <= 0) continue;
      rows.push(row);
    }
  } else {
    for (const candidate of options.candidates) {
      const datedTag = options.holders
        ? holderTagAt(options.holders[candidate], hour)
        : (isSlotKey(candidate) ? null : candidate);
      if (datedTag === null) continue;
      if (options.membership) {
        const inMember = options.membership[candidate]?.some(([first, last]) => first <= hour && hour <= last)
          || options.membership[datedTag]?.some(([first, last]) => first <= hour && hour <= last);
        if (!inMember) continue;
      }
      const rawValue = valueFromNamedAnswer(answer, candidate, { unrecordedReason: 'Not recorded at this hour' });
      rows.push(makeRow(readView, candidate, datedTag, rawValue, config));
    }
    rows.sort(compareExplicitRows);
  }

  return {
    ...baseModel,
    rows,
    ...(!rows.length ? { empty: config.emptyMessage, emptyKind: 'empty' as const } : {}),
  };
}
