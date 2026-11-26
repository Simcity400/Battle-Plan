import { colour } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { auditRow } from '../../families/auditRow.ts';
import { urlArtHandle } from '../../families/art/provider.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TimeChartProjection, TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import type { ChartRowModel } from '../../families/time-chart/model.ts';
import { formatScalar } from '../../families/valueFormat.ts';
import type { ResourceMonthsReply } from '../../generated/dashboard';

interface ResourceTimeOptions {
  readonly extent: readonly [number, number];
  readonly surface: string;
}

function formatNet(value: number): string {
  const size = Math.abs(value);
  const decimals = size >= 100 ? 0 : size >= 10 ? 1 : 2;
  return formatScalar(value, { decimals: 0, maxDecimals: decimals, signed: value !== 0 });
}

function validateResourceMonths(raw: unknown): ResourceMonthsReply {
  const reply = raw as ResourceMonthsReply | null;
  if (!reply || typeof reply !== 'object' || !Array.isArray(reply.resources) || typeof reply.hour !== 'number') {
    throw new Error('The resource months reply is malformed.');
  }
  return reply;
}

const resourceMonthsProvenInterval = (data: unknown, hour: number): readonly [number, number] => {
  const next = (data as ResourceMonthsReply | null)?.nextChange;
  return [hour, next == null ? Infinity : next - 1];
};

export const resourceTimeProjection: TimeChartProjection<ResourceTimeOptions> = registerFamilyProjection<TimeChartProjection<ResourceTimeOptions>>({
  kind: 'resource-balance',
  family: 'time-chart',
  variant: 'columns',
  measureTitle: 'Resource balance',
  project(readView: BoundedReadView, options: ResourceTimeOptions) {
    const reply = readView.panel<ResourceMonthsReply>('months');
    const rows: ChartRowModel[] = (reply?.resources ?? []).map(item => {
      const name = item.resource.name ?? item.resource.token;
      const handle = urlArtHandle(item.resource.icon);
      return {
        id: String(item.resource.id),
        name,
        ...(handle ? { icon: { art: readView.art(handle), label: name } } : {}),
        color: colour(options.surface, item.resource.id),
        valueText: item.net === null ? '' : formatNet(item.net),
        valueMissing: item.missing,
        months: item.months.map(month => ({
          firstHour: month.firstHour,
          lastHour: month.lastHour,
          value: month.net,
          valueText: month.net === null ? '' : formatNet(month.net),
          ...(month.missing ? { reason: month.missing } : {}),
          partial: month.recordedDays < month.days,
        })),
      };
    });
    return {
      kind: 'resource-balance',
      measureTitle: 'Net per day',
      mode: 'rows',
      hour: readView.hour,
      extent: options.extent,
      yAxis: { format: 'number' },
      series: [],
      rows,
      warnings: reply?.warnings ?? [],
      auditRows: rows.map(row => auditRow(row.id, row.valueMissing ?? row.valueText)),
    };
  },
}, 'time');

export const resourceTimeDeclaration: TimeChartWidgetDeclaration<ResourceTimeOptions> = {
  family: 'time-chart',
  kind: 'resource-balance',
  label: 'Resource balance',
  subject: 'resource balance',
  emptyCampaign: 'Load a campaign and choose a country to see its resource balance.',
  icon: 'layers',
  countries: { mode: 'multi', allowSlots: true },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const tag = ctx.countries.tags[0];
    const owner = `resource-balance:time:${ctx.track.source.id}:${ctx.branch.id}:${tag ?? ''}`;
    if (!tag) return { owner, entries: [] };
    return {
      owner,
      entries: [{
        name: 'months',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'point',
          channel: 'panel',
          coverage: { mode: 'keyed', keys: [tag] },
          product: 'resource-balance-months',
          followQuery: null,
          params: { branchId: ctx.branch.id, country: countryFromKey(tag) },
          provenInterval: resourceMonthsProvenInterval,
          dynamicKeys: ['hour'],
          validate: validateResourceMonths,
        },
      }],
    };
  },
  projection: resourceTimeProjection,
  projectionOptions(_ctx, { extent, surface }) {
    return { extent, surface };
  },
};
