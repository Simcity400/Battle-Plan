import { rankProvenInterval, rankReading, validateRankReply } from '../../charts/race/measureRaceData.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import type { GatedHour } from '../../dashboard/regionTime';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { countryRowPresentation, type BarsDataRow, type BarsModel, type BarsProjection, type BarsValueField } from '../../families/bars/model.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { groupSelection } from '../../families/groupSelection.ts';
import { knownValue, notRecorded, partialValue, type Value } from '../../families/value.ts';
import type { GroupSelection, SeriesState } from '../../generated/dashboard';

export const FAMILY = 'casualty-totals';
const RANK = 'dealtAndTaken';
const MEASURES = ['dealt', 'taken', 'unknown', 'partial'] as const;
type Measure = typeof MEASURES[number];
const MOST = 1500;
const PARTIAL = 'Some casualties were recorded without their full detail.';

export interface CasualtiesOptions {
  readonly view: 'butterfly' | 'race';
  readonly includeUnknown: boolean;
  readonly group: boolean;
}

interface Base {
  readonly branchId: string;
  readonly measure: string;
  readonly measures: readonly string[];
  readonly top: number | null;
  readonly countries: readonly string[];
  readonly group: GroupSelection | null;
}

function exactParams(base: Readonly<Record<string, unknown>>, hour: GatedHour): Readonly<Record<string, unknown>> | null {
  const cfg = base as unknown as Base;
  const asked = { branchId: cfg.branchId, family: { kind: FAMILY, measure: cfg.measure }, measures: cfg.measures, hour };
  if (cfg.top != null) return { ...asked, top: cfg.top };
  if (cfg.group) return { ...asked, group: cfg.group };
  return cfg.countries.length ? { ...asked, countries: cfg.countries.map(countryFromKey) } : null;
}

function viewMeasures(view: 'butterfly' | 'race'): readonly Measure[] {
  return view === 'race' ? ['unknown', 'partial'] : MEASURES;
}

function casualtySources(ctx: FamilyPlanContext, view: 'butterfly' | 'race'): SourcePlan {
  const top = ctx.countries.top;
  const keyed = !ctx.countries.all && top == null;
  const group = keyed ? groupSelection(ctx.widget.countries) : null;
  const base: Base = {
    branchId: ctx.branch.id,
    measure: view === 'race' ? 'taken' : RANK,
    measures: viewMeasures(view),
    top: keyed ? null : top ?? MOST,
    countries: keyed && !group ? ctx.countries.tags : [],
    group,
  };
  return {
    owner: `casualties:${view}:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
    entries: [{
      name: 'rank',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'followed',
        coverage: top != null ? { mode: 'population', form: 'top', limit: top } : ctx.countries.all || group ? { mode: 'population', form: 'all' } : { mode: 'keyed', keys: ctx.countries.tags },
        product: 'series-rank',
        exactProduct: 'series-rank',
        followQuery: null,
        params: base as unknown as Readonly<Record<string, unknown>>,
        buildExactParams: exactParams,
        dynamicKeys: ['hour', 'countries'],
        provenInterval: rankProvenInterval,
        validate: validateRankReply,
      },
    }],
  };
}

function raw(state: SeriesState | null): bigint | null {
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
  const panel = readView.panel('rank');
  if (panel === undefined) return { ...base, rows: [] };
  const reply = validateRankReply(panel);
  if (!reply.rows.length && reply.refused) return { ...base, rows: [], empty: reply.refused, emptyKind: 'unavailable' };
  const rows: BarsDataRow[] = reply.rows.map(rankRow => {
    const tag = countryKey(rankRow.country);
    const pres = countryRowPresentation(readView, tag);
    const reading = (measure: Measure) => rankReading(reply, rankRow, measure);
    const reasonOf = (measure: Measure) => reading(measure)?.missing ?? 'Casualties were not recorded at this hour.';
    const partial = raw(reading('partial'));
    const atLeast = partial !== null && partial !== 0n;
    const knownTaken = raw(reading('taken'));
    const unknown = raw(reading('unknown'));
    const taken = options.includeUnknown ? (knownTaken === null || unknown === null ? null : knownTaken + unknown) : knownTaken;
    const takenField = field(taken, atLeast, options.includeUnknown && unknown === null ? reasonOf('unknown') : reasonOf('taken'));
    const shared = { kind: 'data' as const, key: tag, subject: tag, label: pres.label, labelMissing: pres.labelMissing, media: pres.media, ...(pres.color ? { color: pres.color } : {}) };
    if (!butterfly) return { ...shared, value: { ...takenField.field, label: 'Casualties taken' }, numeric: takenField.numeric };
    const dealtField = field(raw(reading('dealt')), atLeast, reasonOf('dealt'));
    return { ...shared, left: { value: takenField.field, numeric: takenField.numeric }, right: { value: dealtField.field, numeric: dealtField.numeric } };
  });
  return { ...base, rows, ...(rows.length ? {} : { empty: options.group ? 'No country of this group at this date.' : 'No casualties at this date.', emptyKind: 'empty' as const }) };
}

const options = (ctx: FamilyPlanContext, view: 'butterfly' | 'race'): CasualtiesOptions => ({
  view,
  includeUnknown: ctx.options.includeUnknown === true,
  group: !ctx.countries.all && ctx.countries.top == null && groupSelection(ctx.widget.countries) !== null,
});

const countries = {
  mode: 'multi' as const,
  labels: true,
  colours: true,
  allowSlots: true,
};

export const casualtiesButterflyProjection: BarsProjection<CasualtiesOptions> = registerFamilyProjection({
  kind: 'casualties', family: 'bars', nextChange: () => Infinity, project,
}, 'butterfly');

export const casualtiesRaceProjection: BarsProjection<CasualtiesOptions> = registerFamilyProjection({
  kind: 'casualties', family: 'bars', nextChange: () => Infinity, project,
}, 'race');

export const casualtiesButterflyDeclaration: BarsWidgetDeclaration<CasualtiesOptions> = {
  family: 'bars', kind: 'casualties', label: 'Casualties taken and dealt', subject: 'casualties',
  emptyCampaign: 'Load a campaign to see casualties.', icon: 'target', countries,
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
