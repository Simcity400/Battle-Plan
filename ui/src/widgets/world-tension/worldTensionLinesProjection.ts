import { auditRow } from '../../families/auditRow.ts';
import { liveKey } from '../../charts/live.ts';
import { categoricalPalette, CHART_MEASURE_COLOURS, nudgeApart, readableColour } from '../../charts/palette.ts';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import { parseColor, toHex } from '../../shell/appearance/color.ts';
import { flagArtHandle } from '../../families/art/provider.ts';
import type { BoundedReadView, FamilyPlanContext, SourcePlan } from '../../families/contract.ts';
import {
  type TimeChartProjection,
  type TimeChartWidgetDeclaration,
} from '../../families/time-chart/declaration.ts';
import type { ChartGap, ChartInlinePoint, TimeChartModel, TimeChartSeriesModel } from '../../families/time-chart/model.ts';
import { branchGapsOf } from '../../families/time-chart/seriesHelpers.ts';
import { formatRecordedPercent } from '../../families/valueFormat.ts';
import type { WorldTensionReply, WorldTensionStackReply } from '../../generated/dashboard';
import { validateWorldTensionHover, validateWorldTensionStack } from './worldTensionProjection.ts';

const TOP_NAMED = 8;

export interface WorldTensionLinesOptions {
  readonly extent: readonly [number, number];
  readonly surface: string;
  readonly gaps: readonly ChartGap[];
}

function formatContributionPct(value: number): string {
  return formatRecordedPercent(value);
}

function resolveLineColors(rawColors: readonly (string | null)[], surface: string): string[] {
  const surfaceRgb = parseColor(surface) ?? { r: 17, g: 17, b: 17 };
  const parsed: { r: number; g: number; b: number }[] = [];
  const indices: number[] = [];
  for (let i = 0; i < rawColors.length; i++) {
    const c = rawColors[i] ? parseColor(readableColour(rawColors[i]!, surface)) : null;
    if (c) {
      parsed.push(c);
      indices.push(i);
    }
  }
  const nudged = parsed.length > 0 ? nudgeApart(parsed, surfaceRgb) : [];
  const fallbacks = categoricalPalette(surface);
  const out: string[] = rawColors.map((_, i) => fallbacks[i % fallbacks.length]!);
  for (let j = 0; j < indices.length; j++) {
    out[indices[j]!] = toHex(nudged[j]!);
  }
  return out;
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
        yAxis: { format: 'percent', fixed: null, min: 0 },
        series: [],
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
        yAxis: { format: 'percent', fixed: null, min: 0 },
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
        yAxis: { format: 'percent', fixed: null, min: 0 },
        series: [],
        ...(gaps.length > 0 ? { gaps } : {}),
        warnings: reply.warnings,
        empty: { message: 'No threat contribution recorded in this view.', kind: 'empty' },
      };
    }
    const endHour = reply.endHour;
    const labelHour = endHour ?? hour;
    const hasNegative = reply.bands.some(band => band.values.some(v => v !== null && v < 0));
    const infos = reply.bands.map(band => {
      if (!band.tag) return { name: 'Unknown', fullName: 'Unknown country', color: null, flagChoice: null };
      const id = readView.who('country', band.tag, labelHour);
      const baseName = id.name.status === 'ready' ? id.name.value : band.tag;
      return {
        name: baseName,
        fullName: band.remembered ? `${baseName} (former)` : baseName,
        color: id.colour?.status === 'ready' ? id.colour.value : null,
        flagChoice: id.flagChoice?.status === 'ready' ? id.flagChoice.value : null,
      };
    });
    const hover = readView.hover<WorldTensionReply>('hover');
    const exactHover = hover && hover.hour <= hour && hover.data ? { hour: hover.hour, data: hover.data } : null;
    const hoverCells = exactHover ? repliedCells(exactHover.data).read() : null;
    const hoverContribMissing = exactHover
      ? (exactHover.data.warnings.find(w => w.startsWith('Country contributions are not known at this hour')) ?? null)
      : null;
    const hoverSlots = exactHover ? new Set(exactHover.data.contributions.map(c => c.slot)) : null;
    const topColors = resolveLineColors(infos.slice(0, TOP_NAMED).map(info => info.color), options.surface);
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
        const min = reply.downsampled ? (band.minValues?.[i] ?? val) : undefined;
        const max = reply.downsampled ? (band.maxValues?.[i] ?? val) : undefined;
        points.push({
          hour: h,
          value: val,
          ...(reply.downsampled ? { min, max, approximate: min !== max } : {}),
        });
      }
      const headValue = endHour !== null && band.endValue !== null && Number.isFinite(band.endValue) ? band.endValue : null;
      let exact: TimeChartSeriesModel['exact'] = null;
      if (exactHover && hoverCells) {
        if (hoverContribMissing) {
          exact = { hour: exactHover.hour, value: null, reason: hoverContribMissing };
        } else if (hoverSlots?.has(band.slot)) {
          const key = liveKey(String(band.slot), 'value');
          const rawVal = hoverCells.get(key);
          if (rawVal === undefined) {
            exact = { hour: exactHover.hour, value: null, reason: hoverCells.missing(key) };
          } else {
            const pct = Number(rawVal) / exactHover.data.contributionScale;
            exact = Number.isFinite(pct)
              ? { hour: exactHover.hour, value: pct }
              : { hour: exactHover.hour, value: null, reason: hoverCells.missing(key) };
          }
        } else {
          exact = { hour: exactHover.hour, value: 0 };
        }
      }
      return {
        id: band.tag ?? `slot:${band.slot}`,
        key: band.tag ?? `slot:${band.slot}`,
        name: info.fullName,
        color: isTop ? topColors[rank]! : CHART_MEASURE_COLOURS.tailLine,
        role: isTop ? 'primary' : 'tail',
        step: true,
        points,
        headValue,
        headHour: labelHour,
        ...(exact ? { exact } : {}),
        ...(flag ? { flag } : {}),
        rank,
        endLabel: isTop && headValue !== null
          ? {
              ...(flag ? { flag } : {}),
              name: info.name,
              fullName: info.fullName,
              value: formatContributionPct(headValue),
            }
          : null,
      };
    });
    const auditRows = series
      .filter(s => s.role === 'primary' && s.headValue !== null && s.headValue !== undefined)
      .map(s => (auditRow(s.id, formatContributionPct(s.headValue!))));
    return {
      kind: 'world-tension',
      measureTitle: 'Contribution to world tension',
      mode: 'lines',
      hour,
      extent: options.extent,
      yAxis: { format: 'percent', fixed: null, ...(hasNegative ? {} : { min: 0 }) },
      series,
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
  countries: { mode: 'multi', labels: true, colours: true, allTagsForLabels: true, everyOnTop: true },
  sources(ctx: FamilyPlanContext, view): SourcePlan {
    const layers = ctx.countries.top != null
      ? Math.max(1, Math.min(1500, ctx.countries.top))
      : ctx.countries.all
        ? 1500
        : Math.max(1, Math.min(1500, ctx.countries.tags.length));
    const countries = ctx.countries.all || ctx.countries.top != null ? null : [...ctx.countries.tags];
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
              width,
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
            params: { branchId: ctx.branch.id },
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
