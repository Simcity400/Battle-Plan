import { formatPower, powerDecimals } from '../../dashboard/powerDecimals.ts';
import { breakdownAtHourModel, breakdownAtHourSource, validateBreakdownAtHour, type BreakdownAtHourWords } from '../../families/breakdown/atHour.ts';
import type { BreakdownWidgetSpec } from '../../families/breakdown/declaration.ts';
import type { BreakdownModel, BreakdownProjection } from '../../families/breakdown/model.ts';
import type { BoundedReadView } from '../../families/plan.ts';
import type { PowerSpendingBreakdownReply } from '../../generated/dashboard';

export interface PoliticalPowerSpendingOptions {
  readonly digits: number;
}

const validate = validateBreakdownAtHour<PowerSpendingBreakdownReply>('political power spending breakdown');

function words(digits: number): BreakdownAtHourWords {
  return {
    unit: 'PP',
    verb: 'Spent',
    since: 'Spent since',
    nothing: 'spending',
    notRecorded: 'Political power spending was not recorded.',
    format: (raw, decimals) => formatPower(raw, digits, decimals),
  };
}

export const ppSpendingProjection: BreakdownProjection<PoliticalPowerSpendingOptions> = {
  family: 'breakdown',
  kind: 'political-power-spending-breakdown',
  project(readView: BoundedReadView, options: PoliticalPowerSpendingOptions): BreakdownModel {
    const reply = readView.panel<PowerSpendingBreakdownReply>('main');
    const lost = reply?.lostAtCapRaw == null ? null : BigInt(reply.lostAtCapRaw);
    const power = (raw: bigint) => `${formatPower(raw, options.digits, reply?.breakdown.decimals ?? 5)} PP`;
    const capped = lost !== null && lost > 0n ? `${power(lost)} lost at the cap.` : null;
    const lostReason = reply?.lostAtCapReason !== reply?.gainedReason ? reply?.lostAtCapReason : null;
    const note = [reply?.gainedReason, reply?.gainedRaw !== null ? capped : null, lostReason].filter((text): text is string => Boolean(text)).join(' ');
    return breakdownAtHourModel(Math.floor(readView.hour), reply, words(options.digits), reply ? {
      label: 'Gained',
      value: reply.gainedRaw === null ? 'Not recorded' : power(BigInt(reply.gainedRaw)),
      ...(reply.gainedRaw !== null && reply.gainedReason ? { partial: true } : {}),
      ...(note ? { note } : {}),
    } : undefined);
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
  sources: ctx => breakdownAtHourSource(ctx, 'power-spending-breakdown', { currency: 'Political', from: ctx.branch.firstHour }, validate),
};
