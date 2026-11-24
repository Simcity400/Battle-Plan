import { seriesStateProvenInterval, validateRankReply } from '../../charts/race/measureRaceData.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import type { GatedHour } from '../../dashboard/regionTime';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { countryRowPresentation, type BarsDataRow, type BarsModel, type BarsProjection, type BarsValueField } from '../../families/bars/model.ts';
import { registerBarsProjection } from '../../families/bars/registry.ts';
import type { BoundedReadView, FamilyPlanContext, SourceEntry, SourcePlan } from '../../families/contract.ts';
import { knownValue, notRecorded, partialValue, type Value } from '../../families/value.ts';
import type { SeriesState } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';

export const FAMILY = 'casualty-totals';
const RANK = 'dealtAndTaken';
const MEASURES = ['dealt', 'taken', 'unknown', 'partial'] as const;
type Measure = typeof MEASURES[number];
const MOST = 1500;
const PARTIAL = 'Some casualties were recorded without their full detail: at least this many.';

export interface CasualtiesOptions {
  readonly view: 'butterfly' | 'race';
  readonly includeUnknown: boolean;
  readonly chosen: readonly string[] | null;
}

interface Base {
  readonly branchId: string;
  readonly measure: string;
  readonly ranked: boolean;
  readonly top: number | null;
  readonly countries: readonly string[];
}

function exactParams(base: Readonly<Record<string, unknown>>, hour: GatedHour): Readonly<Record<string, unknown>> {
  const cfg = base as unknown as Base;
  const family = { kind: FAMILY, measure: cfg.measure };
  if (cfg.ranked && cfg.top != null) return { branchId: cfg.branchId, family, hour, top: cfg.top };
  const reads = cfg.countries.map(tag => ({ country: countryFromKey(tag), hour }));
  return cfg.ranked ? { branchId: cfg.branchId, family, reads, top: cfg.countries.length } : { branchId: cfg.branchId, family, reads };
}

function entry(ctx: FamilyPlanContext, name: string, base: Base): SourceEntry {
  const top = ctx.countries.top;
  return {
    name,
    source: ctx.track.source,
    branch: ctx.branch.id,
    offset: ctx.track.axisOffset,
    read: {
      shape: 'point',
      channel: 'followed',
      coverage: top != null && base.ranked
        ? { mode: 'population', form: 'top', limit: top }
        : ctx.countries.all || top != null ? { mode: 'population', form: 'all' } : { mode: 'keyed', keys: ctx.countries.tags },
      product: 'series-state',
      exactProduct: 'series-state',
      followQuery: null,
      params: base as unknown as Readonly<Record<string, unknown>>,
      buildExactParams: exactParams,
      dynamicKeys: ['hour', 'reads'],
      provenInterval: seriesStateProvenInterval,
      validate: validateRankReply,
    },
  };
}

function casualtySources(ctx: FamilyPlanContext, view: 'butterfly' | 'race'): SourcePlan {
  const top = ctx.countries.top;
  const keyed = !ctx.countries.all && top == null;
  const countries = keyed ? ctx.countries.tags : ctx.countries.every;
  const rankTop = keyed ? null : top ?? MOST;
  const measures: readonly Measure[] = view === 'race' ? ['taken', 'unknown', 'partial'] : MEASURES;
  return {
    owner: `casualties:${view}:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [
      entry(ctx, 'rank', { branchId: ctx.branch.id, measure: view === 'race' ? 'taken' : RANK, ranked: true, top: rankTop, countries }),
      ...measures.map(measure => entry(ctx, measure, { branchId: ctx.branch.id, measure, ranked: false, top: null, countries })),
    ],
  };
}

type Held = ReadonlyMap<string, SeriesState>;

function held(readView: BoundedReadView, name: string): Held | null {
  const panel = readView.panel(name);
  if (panel === undefined) return null;
  return new Map(validateRankReply(panel).states.map(state => [countryKey(state.country), state]));
}

function raw(state: SeriesState | undefined): bigint | null {
  return state?.run ? BigInt(state.run.valueRaw) : null;
}

function field(value: bigint | null, atLeast: boolean, reason: string): { readonly field: BarsValueField; readonly numeric: number | null } {
  if (value === null) return { field: { value: notRecorded(reason) }, numeric: null };
  const shown: Value = atLeast ? partialValue(value, PARTIAL, { atLeast: true }) : knownValue(value);
  return { field: { value: shown, format: 'integer' }, numeric: Math.max(0, Number(value)) };
}

function project(readView: BoundedReadView, options: CasualtiesOptions): BarsModel {
  const butterfly = options.view === 'butterfly';
  const base = butterfly
    ? { form: 'two-sided' as const, hour: readView.hour, animateOrder: true, leftHeader: 'Casualties taken', rightHeader: 'Casualties dealt' }
    : { form: 'ordered' as const, hour: readView.hour, animateOrder: true, showAxis: true, scaleUnit: 'Casualties taken' };
  const refused = readView.refused('rank');
  if (refused) return { ...base, rows: [], empty: refused, emptyKind: 'unavailable' };
  const rankPanel = readView.panel('rank');
  if (rankPanel === undefined) return { ...base, rows: [] };
  const ranked = validateRankReply(rankPanel).states;
  const measures = Object.fromEntries(MEASURES.map(measure => [measure, held(readView, measure)])) as Record<Measure, Held | null>;
  const rows: BarsDataRow[] = [];
  const seen = new Set<string>();
  const missingRows: BarsDataRow[] = [];
  for (const state of ranked) {
    const tag = countryKey(state.country);
    if (seen.has(tag)) continue;
    seen.add(tag);
    const pres = countryRowPresentation(readView, tag);
    const reasonOf = (measure: Measure) => measures[measure]?.get(tag)?.missing ?? state.missing ?? 'Casualties were not recorded at this hour.';
    const partial = raw(measures.partial?.get(tag));
    const atLeast = partial !== null && partial !== 0n;
    const knownTaken = raw(measures.taken?.get(tag));
    const unknown = raw(measures.unknown?.get(tag));
    const taken = options.includeUnknown ? (knownTaken === null || unknown === null ? null : knownTaken + unknown) : knownTaken;
    const takenField = field(taken, atLeast, options.includeUnknown && unknown === null ? reasonOf('unknown') : reasonOf('taken'));
    const row: BarsDataRow = butterfly
      ? (() => {
          const dealtField = field(raw(measures.dealt?.get(tag)), atLeast, reasonOf('dealt'));
          return {
            kind: 'data', key: tag, subject: tag, label: pres.label, labelMissing: pres.labelMissing, media: pres.media,
            ...(pres.color ? { color: pres.color } : {}),
            left: { value: takenField.field, numeric: takenField.numeric },
            right: { value: dealtField.field, numeric: dealtField.numeric },
          };
        })()
      : {
          kind: 'data', key: tag, subject: tag, label: pres.label, labelMissing: pres.labelMissing, media: pres.media,
          ...(pres.color ? { color: pres.color } : {}),
          value: { ...takenField.field, label: 'Casualties taken' },
          numeric: takenField.numeric,
        };
    if (state.run) rows.push(row);
    else missingRows.push(row);
  }
  if (options.chosen) {
    for (const tag of options.chosen) {
      if (seen.has(tag)) continue;
      const pres = countryRowPresentation(readView, tag);
      const value = notRecorded('This country was not recorded at this hour.');
      missingRows.push(butterfly
        ? { kind: 'data', key: tag, subject: tag, label: pres.label, labelMissing: pres.labelMissing, media: pres.media, ...(pres.color ? { color: pres.color } : {}),
            left: { value: { value }, numeric: null }, right: { value: { value }, numeric: null } }
        : { kind: 'data', key: tag, subject: tag, label: pres.label, labelMissing: pres.labelMissing, media: pres.media, ...(pres.color ? { color: pres.color } : {}),
            value: { value }, numeric: null });
    }
  }
  const all = [...rows, ...missingRows];
  return { ...base, rows: all, ...(all.length ? {} : { empty: 'No casualties at this date.', emptyKind: 'empty' as const }) };
}

const options = (ctx: FamilyPlanContext, view: 'butterfly' | 'race'): CasualtiesOptions => ({
  view,
  includeUnknown: ctx.options.includeUnknown === true,
  chosen: !ctx.countries.all && ctx.countries.top == null ? ctx.countries.tags : null,
});

const countries = {
  mode: 'multi' as const,
  labels: true,
  colours: true,
  allowSlots: true,
  requireGroupsWhen: (widget: FamilyPlanContext['widget']) => widget.countries.kind === 'group' && widget.countries.group.type !== 'top',
};

export const casualtiesButterflyProjection: BarsProjection<CasualtiesOptions> = registerBarsProjection({
  kind: 'casualties', family: 'bars', nextChange: () => Infinity, project,
}, 'butterfly');

export const casualtiesRaceProjection: BarsProjection<CasualtiesOptions> = registerBarsProjection({
  kind: 'casualties', family: 'bars', nextChange: () => Infinity, project,
}, 'race');

export const casualtiesButterflyDeclaration: BarsWidgetDeclaration<CasualtiesOptions> = {
  family: 'bars', kind: 'casualties', label: 'Casualties taken and dealt', subject: 'casualties',
  emptyCampaign: 'Load a campaign to see casualties.', icon: 'target', optionFields: OPTIONS, countries,
  sources: ctx => casualtySources(ctx, 'butterfly'),
  projection: casualtiesButterflyProjection,
  projectionOptions: ctx => options(ctx, 'butterfly'),
  jumpToDate: true, keyByBranch: true,
};

export const casualtiesRaceDeclaration: BarsWidgetDeclaration<CasualtiesOptions> = {
  ...casualtiesButterflyDeclaration,
  label: 'Casualties taken',
  sources: ctx => casualtySources(ctx, 'race'),
  projection: casualtiesRaceProjection,
  projectionOptions: ctx => options(ctx, 'race'),
};
