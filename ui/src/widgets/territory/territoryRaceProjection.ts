import { rankSource } from '../../families/seriesRank.ts';
import { validateRankReply } from '../../charts/race/measureRaceData.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { measureOf, ruleOf, territoryMeasureLabel, territoryNote, territorySeriesMeasure, type TerritoryMeasure, type TerritoryRule } from '../../dashboard/territoryMeasure.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import {
  branchFactionIds,
  factionSelectionTop,
  factionTotalsSources,
  groupTitleForSelection,
  isAggregatedSelection,
  isFactionTotalsSelection,
  projectFactionTotalsRace,
  type FactionTotalsRaceOptions,
} from '../../families/bars/factionTotalsRace.ts';
import { barsLabelText, countryRowPresentation, type BarsDataRow, type BarsModel, type BarsProjection } from '../../families/bars/model.ts';
import type { BoundedReadView, FamilyPlanContext, SourceCoverage, SourcePlan } from '../../families/contract.ts';
import { groupSelection } from '../../families/groupSelection.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { valueFromRawBigInt, zeroValue } from '../../families/value.ts';
import { formatScalar } from '../../families/valueFormat.ts';

const ALL_POPULATION_TOP = 10_000;

export interface TerritoryRaceOptions {
  readonly mode: 'countries' | 'factions';
  readonly measure: TerritoryMeasure;
  readonly rule: TerritoryRule;
  readonly measureLabel: string;
  readonly scaleUnit: string;
  readonly note?: string;
  readonly group: boolean;
  readonly factions: FactionTotalsRaceOptions;
}

function unitFor(measure: TerritoryMeasure): string {
  if (measure === 'provinces') return 'Provinces';
  if (measure === 'victoryPoints') return 'Victory points';
  if (measure === 'area') return 'Map cells';
  return 'States';
}

function projectTerritoryCountries(readView: BoundedReadView, options: TerritoryRaceOptions): BarsModel {
  const hour = readView.hour;
  const baseModel = {
    form: 'ordered' as const,
    hour,
    animateOrder: true,
    showAxis: true,
    scaleUnit: options.scaleUnit,
    scaleFormat: { decimals: 0 },
  };
  const refused = readView.refused('main');
  if (refused) {
    return { ...baseModel, rows: [], empty: refused, emptyKind: 'unavailable' };
  }
  const panel = readView.panel('main');
  if (panel === undefined) {
    return { ...baseModel, rows: [] };
  }
  const reply = validateRankReply(panel);
  if (reply.refused !== null) {
    return { ...baseModel, rows: [], empty: reply.refused, emptyKind: 'unavailable' };
  }

  const rows: BarsDataRow[] = [];
  for (const item of reply.rows) {
    const key = item.slot != null ? countryKey(item.slot) : countryKey(item.country);
    const pres = countryRowPresentation(readView, key);
    const who = readView.who('country', key);
    const subjectTag = who.tag?.status === 'ready' ? who.tag.value : (typeof item.country === 'string' ? item.country : key);
    const state = item.readings[0] ?? null;
    const raw = state?.run?.valueRaw ?? null;
    const rawValue = valueFromRawBigInt(raw, { kind: 'notRecorded', reason: state?.missing ?? 'Not recorded at this hour' });
    const value = rawValue.kind === 'known' && Number(rawValue.value) === 0 ? zeroValue() : rawValue;
    const numeric = value.kind === 'known' ? Number(value.value) : value.kind === 'zero' ? 0 : null;
    const tooltipItems: TooltipListItem[] = [
      value.kind === 'notRecorded'
        ? { label: options.measureLabel, missingReason: value.reason }
        : { label: options.measureLabel, value: formatScalar(numeric ?? 0, { decimals: 0 }) },
    ];
    rows.push({
      kind: 'data',
      key,
      subject: subjectTag,
      label: pres.label,
      media: pres.media,
      ...(pres.color ? { color: pres.color } : {}),
      value: { value, format: { decimals: 0 }, label: options.measureLabel },
      numeric,
      tooltip: {
        title: barsLabelText(pres.label),
        hour,
        items: tooltipItems,
        ...(value.kind === 'notRecorded' ? { missingReason: value.reason } : {}),
        ...(options.note ? { note: options.note } : {}),
      },
    });
  }

  const emptyMessage = options.group
    ? 'No country of this group at this date.'
    : `No ${options.measureLabel.toLowerCase()} recorded at this date.`;
  return {
    ...baseModel,
    rows,
    ...(!rows.length ? { empty: emptyMessage, emptyKind: 'empty' as const } : {}),
  };
}

export const territoryRaceProjection: BarsProjection<TerritoryRaceOptions> = registerFamilyProjection({
  kind: 'territory',
  family: 'bars',
  project(readView: BoundedReadView, options: TerritoryRaceOptions): BarsModel {
    if (options.mode === 'factions') {
      return projectFactionTotalsRace(readView, options.factions);
    }
    return projectTerritoryCountries(readView, options);
  },
}, 'race');

function territoryRaceSources(ctx: FamilyPlanContext): SourcePlan {
  const measure = measureOf(ctx.options.measure);
  const rule = ruleOf(ctx.options.rule);
  const seriesMeasure = territorySeriesMeasure(measure, rule);
  if (isFactionTotalsSelection(ctx.widget.countries)) {
    return factionTotalsSources('territory:race', ctx, 'territory', seriesMeasure);
  }
  const top = ctx.countries.top ?? (ctx.countries.all ? Math.max(ALL_POPULATION_TOP, ctx.countries.every.length) : null);
  const group = top == null ? groupSelection(ctx.widget.countries) : null;
  const tags = top == null && !group ? ctx.countries.tags : [];
  const coverage: SourceCoverage = ctx.countries.top != null
    ? { mode: 'population', form: 'top', limit: ctx.countries.top }
    : ctx.countries.all || group
      ? { mode: 'population', form: 'all' }
      : { mode: 'keyed', keys: tags };
  const emptyExplicit = top == null && !group && !tags.length;
  return {
    owner: `territory:race:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${seriesMeasure}`,
    entries: rankSource(ctx, 'main', { kind: 'territory', measure: seriesMeasure }, coverage, tags, top, group),
    allowEmptyFollow: emptyExplicit,
  };
}

export const territoryRaceDeclaration: BarsWidgetDeclaration<TerritoryRaceOptions> = {
  family: 'bars',
  kind: 'territory',
  label: 'Territory',
  subject: 'territory',
  emptyCampaign: 'Load a campaign to see its territory.',
  icon: 'map',
  countries: {
    mode: 'multi',
    allowSlots: true,
    everyOnTop: true,
    factions: true,
  },
  sources: territoryRaceSources,
  projection: territoryRaceProjection,
  projectionOptions: (ctx, extra) => {
    const measure = measureOf(ctx.options.measure);
    const rule = ruleOf(ctx.options.rule);
    const seriesMeasure = territorySeriesMeasure(measure, rule);
    const measureLabel = territoryMeasureLabel(measure, rule);
    const scaleUnit = unitFor(measure);
    const note = territoryNote(measure);
    const useFactions = isFactionTotalsSelection(ctx.widget.countries);
    const aggregated = isAggregatedSelection(ctx.widget.countries);
    const top = ctx.countries.top;
    const all = ctx.countries.all;
    return {
      mode: useFactions ? 'factions' : 'countries',
      measure,
      rule,
      measureLabel,
      scaleUnit,
      ...(note ? { note } : {}),
      group: !useFactions && top == null && !all && groupSelection(ctx.widget.countries) !== null,
      factions: {
        kind: 'territory',
        measure: seriesMeasure,
        measureLabel,
        scaleUnit,
        ...(note ? { note } : {}),
        aggregated,
        groupLabel: groupTitleForSelection(ctx.widget.countries),
        top: factionSelectionTop(ctx.widget.countries),
        factionIds: branchFactionIds(ctx.countries.factions),
        surface: extra.surface,
      },
    };
  },
  unavailable: ctx => {
    if (ctx.widget.countries.kind === 'factions') return null;
    return ctx.countries.reason;
  },
  warnings: (_ctx, followRefused) => (followRefused ? [followRefused] : []),
  jumpToDate: true,
  keyByBranch: true,
};
