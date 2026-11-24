import { rankProvenInterval, validateRankReply } from '../../charts/race/measureRaceData.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import type { GatedHour } from '../../dashboard/regionTime';
import type { GroupSelection, SeriesRankRow } from '../../generated/dashboard';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourcePlan } from '../contract.ts';
import { groupSelection } from '../groupSelection.ts';
import type { TooltipListItem } from '../host/tooltips.ts';
import { knownValue, valueFromRawBigInt, zeroValue, type Value } from '../value.ts';
import { formatValueState, type ValueFormat } from '../valueFormat.ts';
import { countryRowPresentation, type BarsDataRow, type BarsModel } from './model.ts';

const ALL_POPULATION_TOP = 10_000;

interface SeriesRaceBase {
  readonly branchId: string;
  readonly family: { readonly kind: string; readonly measure: string };
  readonly top: number | null;
  readonly countries: readonly string[];
  readonly group: GroupSelection | null;
}

export interface SeriesRaceBaseOptions {
  readonly top: number | null;
  readonly all?: boolean;
  readonly group: boolean;
}

export interface SeriesRaceConfig {
  readonly kind: string;
  readonly family: { readonly kind: string; readonly measure: string };
}

export interface SeriesRaceProjectConfig {
  readonly valueFormat: ValueFormat;
  readonly scaleFormat: ValueFormat;
  readonly scaleUnit: string;
  readonly valueLabel: string;
  readonly emptyMessage: string;
  readonly note?: string;
  readonly positiveOnly?: boolean;
}

function seriesRaceParams(base: Readonly<Record<string, unknown>>, hour: GatedHour): Readonly<Record<string, unknown>> | null {
  const cfg = base as unknown as SeriesRaceBase;
  const asked = { branchId: cfg.branchId, family: cfg.family, hour };
  if (cfg.top != null) return { ...asked, top: cfg.top };
  if (cfg.group) return { ...asked, group: cfg.group };
  return cfg.countries.length ? { ...asked, countries: cfg.countries.map(countryFromKey) } : null;
}

export function seriesRaceSelection(ctx: FamilyPlanContext): SeriesRaceBaseOptions {
  return {
    top: ctx.countries.top,
    all: ctx.countries.all,
    group: !ctx.countries.all && ctx.countries.top == null && groupSelection(ctx.widget.countries) !== null,
  };
}

export function seriesRaceSources(ctx: FamilyPlanContext, config: SeriesRaceConfig): SourcePlan {
  const top = ctx.countries.top ?? (ctx.countries.all ? Math.max(ALL_POPULATION_TOP, ctx.countries.every.length) : null);
  const group = top == null ? groupSelection(ctx.widget.countries) : null;
  const coverage: SourceCoverage = ctx.countries.top != null
    ? { mode: 'population', form: 'top', limit: ctx.countries.top }
    : ctx.countries.all || group
      ? { mode: 'population', form: 'all' }
      : { mode: 'keyed', keys: ctx.countries.tags };
  const base: SeriesRaceBase = {
    branchId: ctx.branch.id,
    family: config.family,
    top,
    countries: top == null && !group ? ctx.countries.tags : [],
    group,
  };
  return {
    owner: `${config.kind}:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${config.family.kind}:${config.family.measure}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage,
        product: 'series-rank',
        exactProduct: 'series-rank',
        followQuery: null,
        params: base as unknown as Readonly<Record<string, unknown>>,
        buildExactParams: seriesRaceParams,
        dynamicKeys: ['hour', 'countries'],
        provenInterval: rankProvenInterval,
        validate: validateRankReply,
      },
    }],
  };
}

export function seriesRaceUnavailable(ctx: FamilyPlanContext): string | null {
  if (!ctx.countries.all && ctx.countries.top == null && groupSelection(ctx.widget.countries) === null && !ctx.countries.tags.length) {
    return 'No countries available for this selection.';
  }
  return null;
}

export function seriesRaceWarnings(followRefused: string | null): readonly string[] {
  return followRefused ? [`Values are not followed here: ${followRefused}.`] : [];
}

function scaleValue(raw: Value<bigint | number>, decimals: number): { readonly value: Value<bigint | number>; readonly numeric: number | null } {
  if (raw.kind === 'notRecorded' || raw.kind === 'notApplicable') return { value: raw, numeric: null };
  if (raw.kind === 'zero') return { value: zeroValue(), numeric: 0 };
  const num = Number(raw.value) / (decimals > 0 ? 10 ** decimals : 1);
  if (decimals === 0) return { value: raw.value === 0 || raw.value === 0n ? zeroValue() : knownValue(raw.value), numeric: num };
  return { value: num === 0 ? zeroValue() : knownValue(num), numeric: num };
}

export function rankRowKey(row: SeriesRankRow): string {
  return countryKey(row.slot ?? row.country);
}

function makeRow(readView: BoundedReadView, rankRow: SeriesRankRow, decimals: number, config: SeriesRaceProjectConfig): BarsDataRow {
  const key = rankRowKey(rankRow);
  const pres = countryRowPresentation(readView, key);
  const state = rankRow.readings[0]!;
  const { value, numeric } = scaleValue(
    valueFromRawBigInt(state.run?.valueRaw ?? null, { kind: 'notRecorded', reason: state.missing ?? 'Not recorded at this hour' }),
    decimals,
  );
  const formatted = formatValueState(value, config.valueFormat);
  const item: TooltipListItem = formatted.kind === 'notRecorded'
    ? { label: config.valueLabel, missingReason: formatted.reason }
    : formatted.kind === 'notApplicable'
      ? { label: config.valueLabel, value: '—' }
      : { label: config.valueLabel, value: formatted.exact ?? formatted.text };
  const identity = readView.who('country', key);
  const tag = identity.tag?.status === 'ready' ? identity.tag.value : typeof rankRow.country === 'string' ? rankRow.country : null;
  return {
    kind: 'data',
    key,
    subject: key,
    label: identity.name.status === 'ready' || tag === null ? pres.label : tag,
    labelMissing: pres.labelMissing,
    media: pres.media,
    ...(pres.color ? { color: pres.color } : {}),
    value: { value, format: config.valueFormat, label: config.valueLabel },
    numeric,
    tooltip: {
      title: pres.labelMissing ? 'Name not recorded' : pres.label,
      hour: readView.hour,
      items: [item],
      ...(config.note ? { note: config.note } : {}),
    },
    meta: { tag: tag ?? key, key },
  };
}

export function projectSeriesRace(readView: BoundedReadView, options: SeriesRaceBaseOptions, config: SeriesRaceProjectConfig): BarsModel {
  const baseModel = {
    form: 'ordered' as const,
    hour: readView.hour,
    animateOrder: true,
    showAxis: true,
    scaleUnit: config.scaleUnit,
    scaleFormat: config.scaleFormat,
  };
  const refused = readView.refused('main');
  if (refused) return { ...baseModel, rows: [], empty: refused, emptyKind: 'unavailable' };
  const panel = readView.panel('main');
  if (panel === undefined) return { ...baseModel, rows: [] };
  const reply = validateRankReply(panel);
  if (!reply.rows.length && reply.refused) return { ...baseModel, rows: [], empty: reply.refused, emptyKind: 'unavailable' };
  const ranked = options.top != null || options.all === true;
  const decimals = reply.measures[0]!.decimals;
  const rows: BarsDataRow[] = [];
  for (const rankRow of reply.rows) {
    const row = makeRow(readView, rankRow, decimals, config);
    if (ranked && config.positiveOnly && row.numeric !== null && row.numeric !== undefined && row.numeric <= 0) continue;
    rows.push(row);
  }
  if (rows.length) return { ...baseModel, rows };
  return { ...baseModel, rows, empty: options.group ? 'No country of this group at this date.' : config.emptyMessage, emptyKind: 'empty' };
}
