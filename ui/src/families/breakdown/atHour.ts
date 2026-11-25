import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { DeclaredProduct } from '../../dashboard/request.ts';
import type { Breakdown } from '../../generated/dashboard';
import type { FamilyPlanContext, SourcePlan } from '../plan.ts';
import { formatDateOnly } from '../valueFormat.ts';
import type { BreakdownComparison, BreakdownModel } from './model.ts';
import { breakdownParts, breakdownTotal, type BreakdownWords } from './product.ts';

export interface BreakdownAtHour {
  readonly from: number;
  readonly hour: number;
  readonly through: number;
  readonly breakdown: Breakdown;
  readonly warnings: readonly string[];
}

export function validateBreakdownAtHour<T extends BreakdownAtHour>(what: string): (raw: unknown) => T {
  return raw => {
    const reply = raw as Partial<BreakdownAtHour> | null;
    if (!reply || typeof reply.hour !== 'number' || typeof reply.through !== 'number' || typeof reply.from !== 'number'
      || !reply.breakdown || !Array.isArray(reply.breakdown.parts) || !Array.isArray(reply.warnings)) {
      throw new Error(`Invalid ${what}`);
    }
    return reply as T;
  };
}

export function breakdownAtHourSource(
  ctx: FamilyPlanContext,
  product: DeclaredProduct,
  params: Readonly<Record<string, unknown>>,
  validate: (raw: unknown) => BreakdownAtHour,
): SourcePlan {
  const subject = ctx.countrySubject;
  const extra = Object.keys(params).length ? `:${JSON.stringify(params)}` : '';
  const query = { branchId: ctx.branch.id, country: subject ? countryFromKey(subject) : null, ...params };
  return {
    owner: `${product}:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${subject ?? ''}${extra}`,
    entries: subject ? [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: { mode: 'population', form: 'parts' },
        product,
        followQuery: { panel: product, ...query },
        params: query,
        validate,
      },
    }] : [],
  };
}

export interface BreakdownAtHourWords extends BreakdownWords {
  readonly since: string;
  readonly nothing: string;
  readonly notRecorded: string;
}

export function breakdownAtHourModel(
  hour: number,
  reply: BreakdownAtHour | undefined,
  words: BreakdownAtHourWords,
  comparison?: BreakdownComparison,
): BreakdownModel {
  const rootLabel = words.verb;
  if (!reply) return { hour, rootLabel, total: { known: false, value: null, text: '', unit: words.unit, verb: words.verb }, parts: [] };
  const total = breakdownTotal(reply.breakdown, words);
  const parts = breakdownParts(reply.breakdown, words);
  const model: BreakdownModel = {
    hour,
    rootLabel,
    periodLabel: `${words.since} ${formatDateOnly(reply.from)}`,
    ...(comparison ? { comparison } : {}),
    total,
    parts,
    ...(reply.warnings.length ? { warnings: reply.warnings } : {}),
  };
  if (parts.length) return model;
  const state = reply.breakdown.total.state;
  if (state === 'notRecorded') {
    return { ...model, empty: 'Not recorded at this time.', emptyDetail: total.reason ?? words.notRecorded, emptyKind: 'unavailable' };
  }
  if (state === 'partial') {
    return { ...model, empty: `No ${words.nothing} recorded at this date.`, emptyDetail: total.reason ?? total.shortReason ?? 'Some hours were not recorded.' };
  }
  return { ...model, empty: `No ${words.nothing} at this date.` };
}
