import { countryKey } from '../../dashboard/countryKey.ts';
import { AREA_UNIT, measureOf, ruleOf, territorySeriesMeasure, type TerritoryMeasure, type TerritoryRule } from '../../dashboard/territoryMeasure.ts';
import { flagChoiceArt } from '../../families/art/provider.ts';
import type { BreakdownWidgetSpec } from '../../families/breakdown/declaration.ts';
import type { BreakdownModel, BreakdownPart, BreakdownProjection } from '../../families/breakdown/model.ts';
import { breakdownParts, breakdownTotal, type BreakdownWords } from '../../families/breakdown/product.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/plan.ts';
import { identityNameOr } from '../../families/table/identityCell.ts';
import { formatRational } from '../../families/valueFormat.ts';
import type { TerritoryShareReply } from '../../generated/dashboard';

export interface TerritoryShareOptions {
  readonly measure: TerritoryMeasure;
  readonly rule: TerritoryRule;
}

const UNITS: Readonly<Record<TerritoryMeasure, string>> = { states: 'states', provinces: 'provinces', victoryPoints: 'victory points', area: AREA_UNIT };
const SLOT = 'slot:';

export function validateTerritoryShare(raw: unknown): TerritoryShareReply {
  const reply = raw as Partial<TerritoryShareReply> | null;
  if (!reply || typeof reply.hour !== 'number' || typeof reply.measure !== 'string' || !reply.breakdown || !Array.isArray(reply.breakdown.parts) || !Array.isArray(reply.warnings)) {
    throw new Error('Invalid territory share');
  }
  return reply as TerritoryShareReply;
}

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

export const territoryShareProjection: BreakdownProjection<TerritoryShareOptions> = {
  family: 'breakdown',
  kind: 'territory',
  project(readView: BoundedReadView, options: TerritoryShareOptions): BreakdownModel {
    const reply = readView.panel<TerritoryShareReply>('main');
    const verb = options.rule === 'owned' ? 'Owned' : 'Controlled';
    const words: BreakdownWords = { unit: UNITS[options.measure], verb, format: (numerator, denominator) => formatRational(numerator, denominator, { maxDecimals: 0 }) };
    const hour = Math.floor(readView.hour);
    if (!reply) return { hour, rootLabel: verb, total: { known: false, value: null, text: '', unit: words.unit, verb }, parts: [] };
    const total = breakdownTotal(reply.breakdown, words);
    const parts = breakdownParts(reply.breakdown, words).map(part => country(readView, part));
    const model: BreakdownModel = {
      hour,
      rootLabel: verb,
      periodLabel: `Share of world ${UNITS[options.measure]}`,
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

function topLimit(ctx: FamilyPlanContext): number | undefined {
  const selection = ctx.widget.countries;
  if (selection.kind === 'all') return undefined;
  return selection.kind === 'group' && selection.group.type === 'top' ? selection.group.count : 10;
}

function sources(ctx: FamilyPlanContext): SourcePlan {
  const measure = measureOf(ctx.options.measure);
  const rule = ruleOf(ctx.options.rule);
  const top = topLimit(ctx);
  const params = { branchId: ctx.branch.id, measure: territorySeriesMeasure(measure, rule), ...(top !== undefined ? { top } : {}) };
  return {
    owner: `territory-share:${ctx.track.source.id}:${ctx.track.source.revision}:${JSON.stringify(params)}`,
    entries: rule === 'occupied' ? [] : [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: { mode: 'population', form: 'parts' },
        product: 'territory-share',
        followQuery: { panel: 'territory-share', ...params },
        params,
        validate: validateTerritoryShare,
      },
    }],
  };
}

export const territoryShare: BreakdownWidgetSpec<TerritoryShareOptions> = {
  kind: 'territory',
  label: 'Territory',
  subject: 'territory',
  emptyCampaign: 'Load a campaign to see its territory.',
  icon: 'map',
  countries: { mode: 'none' },
  unavailable: ctx => (ruleOf(ctx.options.rule) === 'occupied' ? 'World share is only available for controlled or owned territory.' : null),
  projection: territoryShareProjection,
  projectionOptions: ctx => ({
    measure: measureOf(ctx.options.measure),
    rule: ruleOf(ctx.options.rule),
  }),
  sources,
};
