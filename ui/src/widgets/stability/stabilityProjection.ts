import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import {
  displayedHour,
  FutureDataError,
  type ArtHandle,
  type BoundedReadView,
  type FamilyPlanContext,
  type SourcePlan,
} from '../../families/contract.ts';
import type { HolderRun } from '../../families/holders.ts';
import {
  registerTimeChart,
  type TimeChartProjection,
  type TimeChartWidgetDeclaration,
} from '../../families/time-chart/declaration.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import type {
  ChartArtSlot,
  ChartGap,
  ChartHoverDetail,
  ChartMarker,
  ChartMarkerItem,
  TimeChartSeriesModel,
} from '../../families/time-chart/model.ts';
import { notRecordedLabel } from '../../families/valueFormat.ts';
import type { StabilityBreakdownReply } from '../../generated/dashboard';
import {
  assignCountrySeriesColours,
  branchGapsOf,
  countrySeriesIdentity,
  missingSpansOf,
  seriesHeadValue,
  withinMapOf,
} from '../../families/time-chart/seriesHelpers.ts';
import { STABILITY_OPTIONS } from './definition.ts';
import { breakdownRows, linesPerMeasure, type Measure } from './stabilityData.ts';

export interface StabilityRawMarker {
  readonly id: string;
  readonly hour: number;
  readonly kind: 'focus' | 'idea' | 'war' | 'faction' | 'party';
  readonly title: string;
  readonly otherTag?: string;
  readonly warVerb?: 'began' | 'capitulated';
  readonly factionId?: string;
  readonly factionVerb?: 'Founded' | 'Joined' | 'Left';
  readonly fallbackFactionName?: string;
  readonly sprite?: string | null;
  readonly warAlert?: boolean;
}

export interface StabilityOptions {
  readonly tags: readonly string[];
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
  readonly showStability: boolean;
  readonly showWarSupport: boolean;
  readonly zoom: boolean;
  readonly rawMarkers: readonly StabilityRawMarker[];
}

const STABILITY_ICON_HANDLE: ArtHandle = { kind: 'sprite', key: 'GFX_stability_icon', frame: 0 };
const WAR_SUPPORT_ICON_HANDLE: ArtHandle = { kind: 'sprite', key: 'GFX_war_support_icon', frame: 0 };
const WAR_ALERT_ICON_HANDLE: ArtHandle = { kind: 'sprite', key: 'GFX_global_alert_icons_no_backgrounds', frame: 12 };

function formatPct(value: number): string {
  const rounded = Math.round(value * 10) / 10;
  return `${Number.isInteger(rounded) ? Math.round(rounded) : rounded.toFixed(1)}%`;
}

function resolveMarkerTitle(m: StabilityRawMarker, readView: BoundedReadView, limitHour: number): string {
  const at = displayedHour(Math.min(m.hour, limitHour));
  if (m.otherTag && m.warVerb) {
    const who = readView.who('country', m.otherTag, at);
    const other = who.name.status === 'ready' ? who.name.value : m.otherTag;
    return m.warVerb === 'began' ? `War with ${other} began` : `${other} capitulated`;
  }
  if (m.factionId && m.factionVerb) {
    const who = readView.who('faction', m.factionId, at);
    const faction = who.name.status === 'ready' ? who.name.value : (m.fallbackFactionName ?? 'Unnamed faction');
    return `${m.factionVerb} ${faction}`;
  }
  return m.title;
}

const BREAKDOWN_DECIMALS = 5;

function breakdownDetail(readView: BoundedReadView, measures: readonly { readonly key: Measure; readonly label: string }[]): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<StabilityBreakdownReply>('hover');
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover || measures.length === 0) return null;
  const reply = hover.data;
  if (!reply.breakdown) {
    return { hour: hover.hour, items: [{ label: 'Breakdown', missingReason: reply.missing ?? notRecordedLabel(), tone: 'muted' }] };
  }
  const items: TooltipListItem[] = [];
  const perMeasure = linesPerMeasure(measures.length);
  for (const measure of measures) {
    const { rows } = breakdownRows(reply.breakdown, measure.key, BREAKDOWN_DECIMALS, measure.label, perMeasure);
    rows.forEach((row, index) => items.push({
      label: row.label,
      ...(row.value !== undefined ? { value: row.value } : {}),
      ...(row.tone ? { tone: row.tone } : {}),
      ...(row.strong ? { strong: true } : {}),
      ...(index === 0 && items.length > 0 ? { divider: true } : {}),
      ...(index > 0 && !row.strong ? { indent: true } : {}),
    }));
  }
  return { hour: hover.hour, items };
}

function markerPriority(m: StabilityRawMarker): number {
  if (m.warAlert) return 3;
  if (m.kind === 'focus' && m.sprite) return 2;
  if (m.kind === 'idea' && m.sprite) return 1;
  return 0;
}

interface MarkerDay {
  readonly key: number;
  readonly hour: number;
  readonly markers: readonly StabilityRawMarker[];
}

const markerDayCache = new WeakMap<readonly StabilityRawMarker[], readonly MarkerDay[]>();

function markerDays(raw: readonly StabilityRawMarker[]): readonly MarkerDay[] {
  const held = markerDayCache.get(raw);
  if (held) return held;
  const byDay = new Map<number, StabilityRawMarker[]>();
  for (const m of raw) {
    const key = Math.floor(m.hour / 24);
    let list = byDay.get(key);
    if (!list) byDay.set(key, list = []);
    list.push(m);
  }
  const days = [...byDay.entries()].map(([key, list]) => {
    list.sort((a, b) => markerPriority(b) - markerPriority(a) || a.hour - b.hour || a.title.localeCompare(b.title));
    return { key, hour: list[0]!.hour, markers: list };
  }).sort((a, b) => a.hour - b.hour);
  markerDayCache.set(raw, days);
  return days;
}

export const stabilityProjection: TimeChartProjection<StabilityOptions> = registerTimeChart({
  kind: 'stability',
  family: 'series',
  variant: 'line',
  joinMode: 'step',
  measureTitle: 'Stability & War support',
  project(readView: BoundedReadView, options: StabilityOptions) {
    const hour = readView.hour;
    const tags = options.tags;
    const single = tags.length === 1;
    const stabMap = options.showStability ? readView.series('stability') : undefined;
    const warMap = options.showWarSupport ? readView.series('warSupport') : undefined;
    const stabAnswer = options.showStability ? readView.answer('stability') : undefined;
    const warAnswer = options.showWarSupport ? readView.answer('warSupport') : undefined;

    const stabByTag = new Map<string, number | null>();
    const warByTag = new Map<string, number | null>();
    for (let i = 0; i < tags.length; i++) {
      const tag = tags[i]!;
      if (options.showStability) stabByTag.set(tag, seriesHeadValue(stabMap?.get(tag), stabAnswer, i, 5, 100));
      if (options.showWarSupport) warByTag.set(tag, seriesHeadValue(warMap?.get(tag), warAnswer, i, 5, 100));
    }

    const ordered = [...tags].sort((a, b) => {
      const va = stabByTag.get(a) ?? warByTag.get(a) ?? -Infinity;
      const vb = stabByTag.get(b) ?? warByTag.get(b) ?? -Infinity;
      return vb - va;
    });

    const colourMap = single
      ? new Map<string, string>()
      : assignCountrySeriesColours(ordered, options.holders, readView, hour, options.surface);

    const stabIcon: ChartArtSlot | undefined = single && options.showStability
      ? { art: readView.art(STABILITY_ICON_HANDLE), label: 'Stability' }
      : undefined;
    const warIcon: ChartArtSlot | undefined = single && options.showWarSupport
      ? { art: readView.art(WAR_SUPPORT_ICON_HANDLE), label: 'War support' }
      : undefined;

    const series: TimeChartSeriesModel[] = [];
    for (let rank = 0; rank < ordered.length; rank++) {
      const tag = ordered[rank]!;
      const isTail = !single && rank >= 8;
      const { name, flag } = countrySeriesIdentity(tag, options.holders, readView, hour, !single);
      const countryColor = isTail ? 'rgba(245, 245, 245, 0.22)' : (colourMap.get(tag) ?? CHART_MEASURE_COLOURS.stability);

      if (options.showStability) {
        const bounded = stabMap?.get(tag) ?? null;
        const headValue = stabByTag.get(tag) ?? null;
        series.push({
          id: single ? 'stability' : `${tag}:stability`,
          key: single ? 'stability' : `${tag}:stability`,
          name: single ? 'Stability' : `${name} · Stability`,
          color: single ? CHART_MEASURE_COLOURS.stability : countryColor,
          role: isTail ? 'tail' : 'primary',
          step: true,
          area: false,
          bounded,
          valueScale: 100,
          headValue,
          headHour: hour,
          flag,
          icon: single ? stabIcon : undefined,
          missingSpans: missingSpansOf(bounded),
          rank,
          endLabel: isTail || headValue === null
            ? null
            : {
                ...(single ? { icon: stabIcon } : { flag }),
                name: single ? 'Stability' : `${name} · Stability`,
                fullName: single ? 'Stability' : `${name} Stability`,
                value: formatPct(headValue),
              },
        });
      }

      if (options.showWarSupport) {
        const bounded = warMap?.get(tag) ?? null;
        const headValue = warByTag.get(tag) ?? null;
        series.push({
          id: single ? 'warSupport' : `${tag}:warSupport`,
          key: single ? 'warSupport' : `${tag}:warSupport`,
          name: single ? 'War support' : `${name} · War support`,
          color: single ? CHART_MEASURE_COLOURS.warSupport : countryColor,
          role: isTail ? 'tail' : 'primary',
          step: true,
          dash: single ? undefined : [5, 4],
          area: false,
          bounded,
          valueScale: 100,
          headValue,
          headHour: hour,
          flag,
          icon: single ? warIcon : undefined,
          missingSpans: missingSpansOf(bounded),
          rank,
          endLabel: isTail || headValue === null
            ? null
            : {
                ...(single ? { icon: warIcon } : { flag }),
                name: single ? 'War support' : `${name} · War support`,
                fullName: single ? 'War support' : `${name} War support`,
                value: formatPct(headValue),
              },
        });
      }
    }

    const markers: ChartMarker[] = [];
    if (single && options.rawMarkers.length > 0) {
      const days = markerDays(options.rawMarkers);
      for (const day of days) {
        if (day.hour > hour) break;
        if (day.hour < options.extent[0]) continue;
        const iconSlot = (m: StabilityRawMarker): ChartArtSlot | undefined => {
          if (m.warAlert) return { art: readView.art(WAR_ALERT_ICON_HANDLE), label: 'War' };
          if (m.sprite) return { art: readView.art({ kind: 'sprite', key: m.sprite, frame: 0 }), label: m.title };
          return undefined;
        };
        const items: ChartMarkerItem[] = day.markers.map(m => ({
          title: resolveMarkerTitle(m, readView, hour),
          icon: iconSlot(m),
        }));
        markers.push({
          id: `stab-m:${day.key}:${day.markers[0]!.id}`,
          hour: day.hour,
          title: items[0]!.title,
          icon: items[0]!.icon,
          items,
        });
      }
    }

    return {
      kind: 'stability',
      measureTitle: 'Stability & War support',
      mode: 'lines',
      hour,
      extent: options.extent,
      yAxis: {
        format: 'percent',
        fixed: options.zoom ? null : [0, 100],
      },
      series,
      markers: markers.length > 0 ? markers : undefined,
      gaps: options.gaps.length > 0 ? options.gaps : undefined,
      hoverDetail: single
        ? breakdownDetail(readView, [
            ...(options.showStability ? [{ key: 'stability' as const, label: 'Stability' }] : []),
            ...(options.showWarSupport ? [{ key: 'warSupport' as const, label: 'War support' }] : []),
          ])
        : null,
      empty: !options.showStability && !options.showWarSupport
        ? { message: 'Both lines are switched off in the settings.' }
        : null,
      auditRows: series.slice(0, 8).flatMap(s =>
        s.endLabel ? [{ row: s.id, key: s.id, value: s.endLabel.value }] : [],
      ),
    };
  },
});

export const stabilityDeclaration: TimeChartWidgetDeclaration<StabilityOptions> = {
  family: 'series',
  kind: 'stability',
  label: 'Stability & War support',
  subject: 'stability and war support',
  emptyCampaign: 'Load a campaign to see stability and war support.',
  icon: 'chart-line',
  optionFields: STABILITY_OPTIONS,
  countries: { mode: 'multi', labels: true, colours: true, allowSlots: true },
  sources(ctx: FamilyPlanContext): SourcePlan {
    const tags = ctx.countries.tags;
    const within = withinMapOf(ctx.countries.membership);
    const countries = tags.map(countryFromKey);
    const showStab = ctx.options.stability !== false;
    const showWar = ctx.options.warSupport !== false;
    const entries: SourcePlan['entries'][number][] = [];
    if (showStab) {
      const family = { kind: 'country-stability', measure: 'stability' };
      entries.push({
        name: 'stability',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'pixel-binned',
          coverage: { mode: 'keyed', keys: tags },
          family,
          tags,
          ...(within ? { within } : {}),
          marks: false,
          headQuery: tags.length
            ? { product: 'series-state', params: { branchId: ctx.branch.id, family, reads: countries.map(country => ({ country })) } }
            : null,
          exactProduct: 'series-state',
        },
      });
    }
    if (showWar) {
      const family = { kind: 'country-stability', measure: 'warSupport' };
      entries.push({
        name: 'warSupport',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'pixel-binned',
          coverage: { mode: 'keyed', keys: tags },
          family,
          tags,
          ...(within ? { within } : {}),
          marks: false,
          headQuery: tags.length
            ? { product: 'series-state', params: { branchId: ctx.branch.id, family, reads: countries.map(country => ({ country })) } }
            : null,
          exactProduct: 'series-state',
        },
      });
    }
    if (tags.length === 1) {
      entries.push({
        name: 'hover',
        source: ctx.track.source,
        branch: ctx.branch.id,
        offset: ctx.track.axisOffset,
        read: {
          shape: 'exact-hover',
          product: 'country-stability-breakdown',
          params: { branchId: ctx.branch.id, country: countries[0]! },
        },
      });
    }
    return {
      owner: `stability:${ctx.track.source.id}:${ctx.branch.id}:${tags.join(',')}:${showStab ? 1 : 0}:${showWar ? 1 : 0}`,
      entries,
    };
  },
  projection: stabilityProjection,
  projectionOptions(ctx, { extent, surface }) {
    return {
      tags: ctx.countries.tags,
      holders: ctx.countries.holders,
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
      showStability: ctx.options.stability !== false,
      showWarSupport: ctx.options.warSupport !== false,
      zoom: ctx.options.scale === 'zoom',
      rawMarkers: [],
    };
  },
};

export function stabilityDeclarationWith(markers: readonly StabilityRawMarker[]): TimeChartWidgetDeclaration<StabilityOptions> {
  return {
    ...stabilityDeclaration,
    projectionOptions: (ctx, extra) => ({ ...stabilityDeclaration.projectionOptions(ctx, extra), rawMarkers: markers }),
  };
}
