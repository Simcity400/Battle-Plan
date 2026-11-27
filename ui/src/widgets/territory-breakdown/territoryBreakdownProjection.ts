import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { selectedCountryKeys } from '../../dashboard/countrySelection.ts';
import { AREA_UNIT, measureOf, type TerritoryMeasure } from '../../dashboard/territoryMeasure.ts';
import { flagChoiceArt } from '../../families/art/provider.ts';
import type { BreakdownWidgetSpec } from '../../families/breakdown/declaration.ts';
import type { BreakdownModel, BreakdownPart, BreakdownProjection } from '../../families/breakdown/model.ts';
import { breakdownParts, breakdownTotal, type BreakdownWords } from '../../families/breakdown/product.ts';
import { groupSelection } from '../../families/groupSelection.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/plan.ts';
import { formatRational } from '../../families/valueFormat.ts';
import type { TerritoryBreakdownReply } from '../../generated/dashboard';
import { identityNameOr } from '../../families/table/identityCell.ts';

export interface TerritoryBreakdownOptions {
  readonly measure: TerritoryMeasure;
  readonly rule: 'controlled' | 'owned';
  readonly chosen: boolean;
}

const UNITS: Readonly<Record<TerritoryMeasure, string>> = { states: 'states', provinces: 'provinces', victoryPoints: 'victory points', area: AREA_UNIT };

export function validateTerritoryBreakdown(raw: unknown): TerritoryBreakdownReply {
  const reply = raw as Partial<TerritoryBreakdownReply> | null;
  if (!reply || typeof reply.hour !== 'number' || !reply.breakdown || !Array.isArray(reply.breakdown.parts) || !Array.isArray(reply.warnings)) {
    throw new Error('Invalid territory breakdown');
  }
  return reply as TerritoryBreakdownReply;
}

const SLOT = 'slot:';

function country(readView: BoundedReadView, part: BreakdownPart): BreakdownPart {
  if (part.neutral || !part.key.startsWith(SLOT)) return part;
  const key = countryKey(Number(part.key.slice(SLOT.length)));
  const identity = readView.who('country', key);
  return {
    ...part,
    name: identityNameOr(readView, 'country', key, part.name),
    flag: identity.tag?.status === 'ready' ? identity.tag.value : part.name,
    flagArt: flagChoiceArt(readView, identity.flagChoice),
    ...(identity.colour?.status === 'ready' ? { color: identity.colour.value } : {}),
  };
}

export const territoryBreakdownProjection: BreakdownProjection<TerritoryBreakdownOptions> = {
  family: 'breakdown',
  kind: 'territory-breakdown',
  project(readView: BoundedReadView, options: TerritoryBreakdownOptions): BreakdownModel {
    const reply = readView.panel<TerritoryBreakdownReply>('main');
    const verb = options.rule === 'owned' ? 'Owned' : 'Controlled';
    const words: BreakdownWords = { unit: UNITS[options.measure], verb, format: (numerator, denominator) => formatRational(numerator, denominator, { maxDecimals: options.measure === 'states' ? 2 : 0 }) };
    const hour = Math.floor(readView.hour);
    if (!reply) return { hour, rootLabel: verb, total: { known: false, value: null, text: '', unit: words.unit, verb }, parts: [] };
    const total = breakdownTotal(reply.breakdown, words);
    const parts = breakdownParts(reply.breakdown, words).map(part => country(readView, part));
    const model: BreakdownModel = {
      hour,
      rootLabel: verb,
      periodLabel: options.chosen ? `${verb} by the chosen countries` : `${verb} by every country`,
      total,
      parts,
      ...(reply.warnings.length ? { warnings: reply.warnings } : {}),
    };
    if (parts.length) return model;
    if (reply.breakdown.total.state === 'notRecorded') {
      return { ...model, empty: 'Not recorded at this time.', emptyDetail: total.reason ?? 'Territory was not recorded.', emptyKind: 'unavailable' };
    }
    return { ...model, empty: 'No territory at this date.' };
  },
};

function chosen(ctx: FamilyPlanContext): Readonly<Record<string, unknown>> {
  const selection = ctx.widget.countries;
  if (selection.kind === 'all') return {};
  const group = groupSelection(selection);
  if (group) return { group };
  return { countries: selectedCountryKeys(selection, ctx.track).map(countryFromKey) };
}

function sources(ctx: FamilyPlanContext): SourcePlan {
  const measure = measureOf(ctx.options.measure);
  const rule = ctx.options.rule === 'owned' ? 'owned' : 'controlled';
  const params = { branchId: ctx.branch.id, measure, rule, ...chosen(ctx) };
  return {
    owner: `territory-breakdown:${ctx.track.source.id}:${ctx.track.source.revision}:${JSON.stringify(params)}`,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: { mode: 'population', form: 'parts' },
        product: 'territory-breakdown',
        followQuery: { panel: 'territory-breakdown', ...params },
        params,
        validate: validateTerritoryBreakdown,
      },
    }],
  };
}

export const territoryBreakdown: BreakdownWidgetSpec<TerritoryBreakdownOptions> = {
  kind: 'territory-breakdown',
  label: 'Territory breakdown',
  subject: 'territory',
  emptyCampaign: 'Load a campaign to see its territory breakdown.',
  icon: 'sunburst',
  countries: { mode: 'filter', content: 'hour' },
  projection: territoryBreakdownProjection,
  projectionOptions: ctx => ({
    measure: measureOf(ctx.options.measure),
    rule: ctx.options.rule === 'owned' ? 'owned' : 'controlled',
    chosen: ctx.widget.countries.kind !== 'all',
  }),
  sources,
};
