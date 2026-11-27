import { liveKey } from '../../charts/live.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { defaultCountryKey, selectedCountryKeys } from '../../dashboard/countrySelection.ts';
import { contentKey, contentSet } from '../../dashboard/options.ts';
import type { BarsWidgetDeclaration } from '../../families/bars/declaration.ts';
import { countryRowPresentation, type BarsDataRow, type BarsModel, type BarsProjection, type BarsSideMeasure } from '../../families/bars/model.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { regionLabel } from '../../families/heat/scale.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { knownValue, notRecorded, zeroValue } from '../../families/value.ts';
import type { AirStatisticsChannel } from '../../generated/dashboard';
import { channelIndex, DEFAULT_CHANNEL, statisticName, validateAirStatistics } from './airStatisticsData.ts';

export interface AirStatisticsOptions {
  readonly channel: string;
  readonly country: string | null;
  readonly ratio: boolean;
}

const NOT_READ = 'Not recorded at this hour';

function digits(divisor: number): number {
  return Math.max(0, Math.round(Math.log10(divisor)));
}

function side(raw: number | bigint | undefined, reason: string, channel: AirStatisticsChannel): BarsSideMeasure {
  if (raw === undefined) return { value: { value: notRecorded(reason) }, numeric: null };
  const exact = BigInt(raw);
  const value = exact === 0n ? zeroValue() : knownValue(exact);
  return { value: { value, format: { decimals: digits(channel.divisor) } }, numeric: Math.abs(Number(exact)) / channel.divisor };
}

function ratio(ours: number | bigint | undefined, enemy: number | bigint | undefined): string | null {
  if (ours === undefined || enemy === undefined) return null;
  const mine = BigInt(ours), theirs = BigInt(enemy);
  if (mine <= 0n || theirs < 0n) return null;
  const hundredths = (theirs * 100n + mine / 2n) / mine;
  return `${hundredths / 100n}.${String(hundredths % 100n).padStart(2, '0')}`;
}

function project(readView: BoundedReadView, options: AirStatisticsOptions): BarsModel {
  const base = { form: 'two-sided' as const, hour: readView.hour, animateOrder: true };
  const refused = readView.refused('main');
  if (refused) return { ...base, rows: [], empty: refused, emptyKind: 'unavailable' };
  const panel = readView.panel('main');
  if (panel === undefined) return { ...base, rows: [] };
  const reply = validateAirStatistics(panel);
  const channel = reply.channels.find(item => item.id === options.channel);
  if (reply.missing !== null) return { ...base, rows: [], empty: reply.missing, emptyKind: 'unavailable', warnings: reply.warnings };
  if (!channel) return { ...base, rows: [], empty: 'This statistic is not in this recording.', emptyKind: 'unavailable', warnings: reply.warnings };
  const live = readView.live('main');
  const colour = options.country ? countryRowPresentation(readView, options.country).color : undefined;
  const rows: BarsDataRow[] = reply.regions.map(region => {
    const named = regionLabel(readView, region.region, region.name);
    const oursKey = liveKey(region.region, 'ours'), enemyKey = liveKey(region.region, 'enemy');
    const ours = live?.raw?.(oursKey) ?? (live?.get(oursKey) as number | undefined);
    const enemy = live?.raw?.(enemyKey) ?? (live?.get(enemyKey) as number | undefined);
    const shown = options.ratio ? ratio(ours, enemy) : null;
    return {
      kind: 'data', key: region.region, subject: region.region, label: named.value,
      ...(colour ? { color: colour } : {}),
      left: side(ours, live?.missing(oursKey) ?? NOT_READ, channel),
      right: side(enemy, live?.missing(enemyKey) ?? NOT_READ, channel),
      ...(shown ? { readout: { primary: shown } } : {}),
    };
  });
  return {
    ...base,
    leftHeader: statisticName(channel.ours),
    rightHeader: statisticName(channel.enemy),
    ...(options.ratio ? { readoutHeader: 'Ratio' } : {}),
    rows,
    ...(rows.length ? {} : { empty: 'No air statistics at this date.', emptyKind: 'empty' as const }),
    warnings: reply.warnings,
  };
}

export const airStatisticsProjection: BarsProjection<AirStatisticsOptions> = registerFamilyProjection({
  kind: 'air-statistics', family: 'bars', nextChange: () => Infinity, project,
});

const chosenChannel = (ctx: FamilyPlanContext) => ctx.options.channel ? contentKey(String(ctx.options.channel)) : DEFAULT_CHANNEL;
const countryOf = (ctx: FamilyPlanContext) => selectedCountryKeys(ctx.widget.countries, ctx.track)[0] ?? defaultCountryKey(ctx.track) ?? null;

function sources(ctx: FamilyPlanContext): SourcePlan {
  const country = countryOf(ctx);
  const channel = channelIndex(chosenChannel(ctx));
  const regions = contentSet(ctx.options.regions).map(contentKey).map(Number).filter(Number.isInteger);
  const theatres = contentSet(ctx.options.theatres).map(contentKey);
  const top = Math.max(1, Number(ctx.options.regionCount) || 10);
  const owner = `air-statistics:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}:${JSON.stringify([country, channel, regions, theatres, top])}`;
  if (!country) return { owner, entries: [], unavailable: 'Choose a country in the widget’s header.' };
  if (channel === null) return { owner, entries: [], unavailable: 'This statistic is not in this recording.' };
  const params = { branchId: ctx.branch.id, country: countryFromKey(country), channel, regions, theatres, ...(regions.length ? {} : { top }) };
  return {
    owner,
    entries: [{
      name: 'main',
      source: ctx.track.source,
      branch: ctx.branch.id,
      offset: ctx.track.axisOffset,
      read: {
        shape: 'point',
        channel: 'panel',
        coverage: regions.length ? { mode: 'population', form: 'all' } : { mode: 'population', form: 'top', limit: top },
        product: 'air-statistics',
        followQuery: { panel: 'air-statistics', ...params },
        params,
        validate: validateAirStatistics,
      },
    }],
  };
}

export const airStatisticsDeclaration: BarsWidgetDeclaration<AirStatisticsOptions> = {
  family: 'bars',
  kind: 'air-statistics',
  label: 'Air statistics by region',
  subject: 'air statistics',
  emptyCampaign: 'Load a campaign to see air statistics.',
  icon: 'plane',
  countries: { mode: 'none' },
  sources,
  projection: airStatisticsProjection,
  projectionOptions: ctx => ({ channel: chosenChannel(ctx), country: countryOf(ctx), ratio: ctx.options.ratio !== false }),
  jumpToDate: true,
  keyByBranch: true,
};
