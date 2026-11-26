import { auditRow } from '../../families/auditRow.ts';
import { countryKey } from '../../dashboard/countryKey.ts';
import { resolveLineColors, tailColour } from '../../charts/palette.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { flagArtHandle } from '../../families/art/provider.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import { groupSelection } from '../../families/groupSelection.ts';
import {
  type TimeChartProjection,
  type TimeChartWidgetDeclaration,
} from '../../families/time-chart/declaration.ts';
import type { ChartGap, ChartInlinePoint, TimeChartModel, TimeChartSeriesModel } from '../../families/time-chart/model.ts';
import { branchGapsOf } from '../../families/time-chart/seriesHelpers.ts';
import { formatRational, formatRecordedPercent } from '../../families/valueFormat.ts';
import type { WorldTensionStackReply } from '../../generated/dashboard';
import { tensionHover, validateWorldTensionHover, validateWorldTensionStack } from './worldTensionProjection.ts';
import { identityName } from '../../families/table/identityCell.ts';

const TOP_NAMED = 8;

export interface WorldTensionLinesOptions {
  readonly extent: readonly [number, number];
  readonly surface: string;
  readonly gaps: readonly ChartGap[];
}

function parseRawInt(raw: string | null | undefined): bigint | null {
  return typeof raw === 'string' && /^-?\d+$/.test(raw) ? BigInt(raw) : null;
}

function formatContributionPct(value: number, rawStr: string | null | undefined, scale: number): string {
  const parsed = parseRawInt(rawStr);
  if (parsed !== null && scale > 0) {
    return formatRational(parsed, BigInt(scale), { maxDecimals: 5, percent: true });
  }
  return formatRecordedPercent(value);
}

export const worldTensionLinesProjection: TimeChartProjection<WorldTensionLinesOptions> = registerFamilyProjection({
  kind: 'world-tension',
  family: 'time-chart',
  variant: 'line',
  joinMode: 'step',
  measureTitle: 'Contribution to world tension',
  project(readView: BoundedReadView, options: WorldTensionLinesOptions): TimeChartModel {
    const hour = readView.hour;
    const reply = readView.panel<WorldTensionStackReply>('main');
    if (!reply) {
      return {
        kind: 'world-tension',
        measureTitle: 'Contribution to world tension',
        mode: 'lines',
        hour,
        extent: options.extent,
        yAxis: { format: 'signedPercent', fixed: null, min: 0 },
        series: [],
      };
    }
    if (reply.refused) {
      return {
        kind: 'world-tension',
        measureTitle: 'Contribution to world tension',
        mode: 'lines',
        hour,
        extent: options.extent,
        yAxis: { format: 'signedPercent', fixed: null, min: 0 },
        series: [],
        warnings: reply.warnings,
        empty: { message: reply.refused, kind: 'unavailable' },
      };
    }
    const gaps: ChartGap[] = [
      ...options.gaps,
      ...reply.missing.map(sp => ({ firstHour: sp.firstHour, lastHour: sp.lastHour, reason: sp.reason })),
    ];
    if (reply.hours.length === 0) {
      const reason = reply.missing[0]?.reason ?? 'World tension contributions were not recorded.';
      return {
        kind: 'world-tension',
        measureTitle: 'Contribution to world tension',
        mode: 'lines',
        hour,
        extent: options.extent,
        yAxis: { format: 'signedPercent', fixed: null, min: 0 },
        series: [],
        warnings: reply.warnings,
        empty: { message: reason, kind: 'unavailable' },
      };
    }
    if (reply.bands.length === 0) {
      return {
        kind: 'world-tension',
        measureTitle: 'Contribution to world tension',
        mode: 'lines',
        hour,
        extent: options.extent,
        yAxis: { format: 'signedPercent', fixed: null, min: 0 },
        series: [],
        ...(gaps.length > 0 ? { gaps } : {}),
        warnings: reply.warnings,
        empty: { message: 'No threat contribution recorded in this view.', kind: 'empty' },
      };
    }
    const endHour = reply.endHour;
    const labelHour = endHour ?? hour;
    const hasNegative = reply.bands.some(band => [...band.values, ...(band.minValues ?? [])].some(v => v !== null && v < 0));
    const infos = reply.bands.map(band => {
      if (!band.tag) return { name: 'Unknown', fullName: 'Unknown country', color: null, flagChoice: null };
      const id = readView.who('country', band.tag, hour);
      const baseName = identityName(readView, 'country', band.tag, hour);
      return {
        name: baseName,
        fullName: band.remembered ? `${baseName} (former)` : baseName,
        color: id.colour?.status === 'ready' ? id.colour.value : null,
        flagChoice: id.flagChoice?.status === 'ready' ? id.flagChoice.value : null,
      };
    });
    const hover = tensionHover(readView);
    const topColors = resolveLineColors(reply.bands.slice(0, TOP_NAMED).map((band, i) => ({ key: band.slot, color: infos[i]!.color })), options.surface);
    const scaleBig = reply.contributionScale > 0 ? BigInt(reply.contributionScale) : null;
    const endTexts = new Map<string, string>();
    const series: TimeChartSeriesModel[] = reply.bands.map((band, rank) => {
      const isTop = rank < TOP_NAMED;
      const info = infos[rank]!;
      const flag = info.flagChoice
        ? { art: readView.art(flagArtHandle(info.flagChoice, 'panel')), tag: band.tag ?? undefined, label: info.name }
        : undefined;
      const points: ChartInlinePoint[] = [];
      for (let i = 0; i < reply.hours.length; i++) {
        const h = reply.hours[i]!;
        if (h > hour) break;
        const val = band.values[i] ?? null;
        const rawInt = !reply.downsampled && scaleBig ? parseRawInt(band.valuesRaw?.[i]) : null;
        const min = reply.downsampled ? (band.minValues?.[i] ?? val) : undefined;
        const max = reply.downsampled ? (band.maxValues?.[i] ?? val) : undefined;
        points.push({
          hour: h,
          value: val,
          ...(rawInt !== null && scaleBig ? { raw: rawInt, denominator: scaleBig } : {}),
          ...(reply.downsampled ? { min, max, approximate: min !== max } : {}),
        });
      }
      const headValue = endHour !== null && band.endValue !== null && Number.isFinite(band.endValue) ? band.endValue : null;
      const headText = headValue !== null ? formatContributionPct(headValue, band.endRaw, reply.contributionScale) : null;
      endTexts.set(`slot:${band.slot}`, headText ?? band.endMissing ?? 'Not recorded at this hour.');
      let exact: TimeChartSeriesModel['exact'] = null;
      if (hover) {
        const reading = hover.at(band);
        exact = reading.raw !== undefined && hover.detail.contributionScale > 0
          ? { hour: hover.hour, value: Number(reading.raw) / hover.detail.contributionScale,
              raw: reading.raw, denominator: BigInt(hover.detail.contributionScale) }
          : { hour: hover.hour, value: null, reason: reading.reason ?? 'Contribution scale was not recorded.' };
      }
      return {
        id: `slot:${band.slot}`,
        key: `slot:${band.slot}`,
        name: info.fullName,
        color: isTop ? topColors.get(band.slot)! : tailColour(options.surface),
        role: isTop ? 'primary' : 'tail',
        step: true,
        points,
        headValue,
        headHour: labelHour,
        ...(exact ? { exact } : {}),
        ...(flag ? { flag } : {}),
        rank,
        endLabel: isTop && headText !== null
          ? {
              ...(flag ? { flag } : {}),
              name: info.name,
              fullName: info.fullName,
              value: headText,
            }
          : null,
      };
    });
    const auditRows = series.map(s => auditRow(s.id, endTexts.get(s.id)!));
    return {
      kind: 'world-tension',
      measureTitle: 'Contribution to world tension',
      mode: 'lines',
      hour,
      extent: options.extent,
      yAxis: { format: 'signedPercent', fixed: null, ...(hasNegative ? {} : { min: 0 }) },
      series,
      drawnKeys: { main: reply.bands.map(band => countryKey(band.slot)) },
      ...(gaps.length > 0 ? { gaps } : {}),
      warnings: reply.warnings,
      auditRows,
    };
  },
}, 'lines');

export const worldTensionLinesDeclaration: TimeChartWidgetDeclaration<WorldTensionLinesOptions> = {
  family: 'time-chart',
  kind: 'world-tension',
  label: 'Contributions over time',
  subject: 'world tension contributions',
  emptyCampaign: 'Load a campaign to see world tension contributions.',
  icon: 'chart-line',
  countries: { mode: 'multi', everyOnTop: true },
  sources(ctx: FamilyPlanContext, view): SourcePlan {
    const top = ctx.countries.top ?? null;
    const keyed = !ctx.countries.all && top == null;
    const group = keyed ? groupSelection(ctx.widget.countries) : null;
    const layers = top != null
      ? Math.max(1, Math.min(1500, top))
      : ctx.countries.all || group
        ? 1500
        : Math.max(1, Math.min(1500, ctx.countries.tags.length));
    const countries = keyed && !group ? [...ctx.countries.tags] : null;
    const width = Math.max(240, Math.min(2400, Math.ceil(view.width ?? 1024)));
    return {
      owner: `world-tension-lines:${ctx.track.source.id}:${ctx.track.source.revision}:${ctx.branch.id}`,
      entries: [
        {
          name: 'main',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'branch-to-hour',
            coverage: { mode: 'population', form: 'all' },
            product: 'world-tension-stack',
            followQuery: null,
            params: {
              branchId: ctx.branch.id,
              layers,
              values: 'recorded',
              countries,
              top,
              group,
              width,
              includeStack: true,
              includeEvents: false,
            },
            dynamicKeys: ['firstHour', 'lastHour', 'width'],
            validate: validateWorldTensionStack,
            provenInterval: (data, _requested, hour) => {
              const reply = data as WorldTensionStackReply;
              return reply.nextChangeHour !== null ? [hour, reply.nextChangeHour - 1] : [hour, Infinity];
            },
          },
        },
        {
          name: 'hover',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'exact-hover',
            product: 'world-tension',
            countriesOf: 'main',
            params: { branchId: ctx.branch.id, values: 'recorded', group },
            validate: validateWorldTensionHover,
          },
        },
      ],
    };
  },
  projection: worldTensionLinesProjection,
  projectionOptions(ctx, { extent, surface }) {
    return {
      extent,
      surface,
      gaps: branchGapsOf(ctx.branch),
    };
  },
};
