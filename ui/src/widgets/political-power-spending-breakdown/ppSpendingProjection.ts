import { countryFromKey } from '../../dashboard/countryKey.ts';
import { formatPower, powerDecimals } from '../../dashboard/powerDecimals.ts';
import type { BreakdownWidgetSpec } from '../../families/breakdown/declaration.ts';
import type { BreakdownModel, BreakdownProjection } from '../../families/breakdown/model.ts';
import { breakdownParts, breakdownTotal, type BreakdownWords } from '../../families/breakdown/product.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/plan.ts';
import { formatDateOnly } from '../../families/valueFormat.ts';
import type { PowerSpendingBreakdownReply } from '../../generated/dashboard';

export interface PoliticalPowerSpendingOptions {
  readonly digits: number;
}

function validate(raw: unknown): PowerSpendingBreakdownReply {
  const reply = raw as Partial<PowerSpendingBreakdownReply> | null;
  if (!reply || typeof reply.hour !== 'number' || typeof reply.through !== 'number' || !reply.breakdown || !Array.isArray(reply.breakdown.parts)) {
    throw new Error('Invalid political power spending breakdown');
  }
  return reply as PowerSpendingBreakdownReply;
}

function words(digits: number): BreakdownWords {
  return { unit: 'PP', verb: 'Spent', format: raw => formatPower(raw, digits) };
}

export const ppSpendingProjection: BreakdownProjection<PoliticalPowerSpendingOptions> = {
  family: 'breakdown',
  kind: 'political-power-spending-breakdown',
  project(readView: BoundedReadView, options: PoliticalPowerSpendingOptions): BreakdownModel {
    const hour = Math.floor(readView.hour);
    const said = words(options.digits);
    const reply = readView.panel<PowerSpendingBreakdownReply>('main');
    if (!reply) return { hour, rootLabel: 'Spent', total: { known: false, value: null, text: '', unit: said.unit, verb: said.verb }, parts: [] };
    const total = breakdownTotal(reply.breakdown, said);
    const parts = breakdownParts(reply.breakdown, said);
    const lost = reply.lostAtCapRaw === null ? null : BigInt(reply.lostAtCapRaw);
    const model: BreakdownModel = {
      hour,
      rootLabel: 'Spent',
      periodLabel: `Spent since ${formatDateOnly(reply.from)}`,
      comparison: {
        label: 'Gained',
        value: reply.gainedRaw === null ? 'Not recorded' : `${formatPower(BigInt(reply.gainedRaw), options.digits)} PP`,
        ...(reply.gainedRaw === null && reply.gainedReason
          ? { note: reply.gainedReason }
          : lost !== null && lost > 0n ? { note: `${formatPower(lost, options.digits)} PP lost at the cap.` } : {}),
      },
      total,
      parts,
      ...(reply.warnings.length ? { warnings: reply.warnings } : {}),
    };
    if (parts.length) return model;
    if (reply.breakdown.total.state === 'NotRecorded') {
      return { ...model, empty: 'Not recorded at this time.', emptyDetail: total.reason ?? 'Political power spending was not recorded.', emptyKind: 'unavailable' };
    }
    return { ...model, empty: 'No spending at this date.' };
  },
};

export const ppSpendingBreakdown: BreakdownWidgetSpec<PoliticalPowerSpendingOptions> = {
  kind: 'political-power-spending-breakdown',
  label: 'Political power spending breakdown',
  subject: 'political power spending',
  emptyCampaign: 'Load a campaign to see political power spending.',
  icon: 'sunburst',
  countries: { mode: 'single', choose: 'Choose a country to see its political power spending.' },
  projection: ppSpendingProjection,
  projectionOptions: ctx => ({ digits: powerDecimals(ctx.options) }),
  sources: (ctx: FamilyPlanContext): SourcePlan => {
    const tag = ctx.countryTag;
    return {
      owner: `political-power-spending-breakdown:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${tag ?? ''}`,
      entries: tag ? [{
        name: 'main',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'followed',
          coverage: { mode: 'population', form: 'parts' },
          product: 'power-spending-breakdown',
          followQuery: null,
          params: { branchId: ctx.branch.id, country: countryFromKey(tag), currency: 'Political', from: ctx.branch.firstHour },
          hourParam: 'hour',
          provenInterval: (data, hour) => {
            const reply = data as PowerSpendingBreakdownReply;
            return reply.hour <= hour && hour <= reply.through ? [reply.hour, reply.through] : [hour, hour];
          },
          validate,
        },
      }] : [],
    };
  },
};
