import { CHART_MEASURE_COLOURS } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import {
  displayedHour,
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
import type {
  ChartArtSlot,
  ChartGap,
  ChartMarker,
  ChartMarkerItem,
  TimeChartSeriesModel,
} from '../../families/time-chart/model.ts';
import {
  assignCountrySeriesColours,
  branchGapsOf,
  countrySeriesIdentity,
  missingSpansOf,
  seriesHeadValue,
  withinMapOf,
} from '../../families/time-chart/seriesHelpers.ts';
import { STABILITY_OPTIONS } from './definition.ts';

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
  readonly iconUrl?: string | null;
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

function markerPriority(m: StabilityRawMarker): number {
  if (m.warAlert) return 3;
  if (m.kind === 'focus' && m.iconUrl) return 2;
  if (m.kind === 'idea' && m.iconUrl) return 1;
  return 0;
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
      const byDay = new Map<number, StabilityRawMarker[]>();
      for (const m of options.rawMarkers) {
        if (m.hour > hour || m.hour < options.extent[0]) continue;
        const dayKey = Math.floor(m.hour / 24);
        let list = byDay.get(dayKey);
        if (!list) byDay.set(dayKey, list = []);
        list.push(m);
      }
      const days = [...byDay.keys()].sort((a, b) => a - b);
      for (const dayKey of days) {
        const rawList = byDay.get(dayKey)!;
        rawList.sort((a, b) => markerPriority(b) - markerPriority(a) || a.hour - b.hour || a.title.localeCompare(b.title));
        const lead = rawList[0]!;
        const iconSlot = (m: StabilityRawMarker): ChartArtSlot | undefined => {
          if (m.warAlert) return { art: readView.art(WAR_ALERT_ICON_HANDLE), label: 'War' };
          if (m.iconUrl) return { art: readView.art({ kind: 'url', url: m.iconUrl }), label: m.title };
          return undefined;
        };
        const items: ChartMarkerItem[] = rawList.map(m => ({
          title: resolveMarkerTitle(m, readView, hour),
          icon: iconSlot(m),
        }));
        markers.push({
          id: `stab-m:${dayKey}:${lead.id}`,
          hour: lead.hour,
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
      empty: !options.showStability && !options.showWarSupport
        ? { message: 'Both lines are switched off in the settings.' }
        : null,
      auditRows: series.slice(0, 8).flatMap(s =>
        s.endLabel ? [{ row: s.id, key: s.id, value: s.endLabel.value }] : [],
      ),
    };
  },
});

export const stabilityDeclaration: TimeChartWidgetDeclaration<StabilityOptions, unknown, readonly StabilityRawMarker[]> = {
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
  projectionOptions(ctx, { extent, surface, extra }) {
    return {
      tags: ctx.countries.tags,
      holders: ctx.countries.holders,
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
      showStability: ctx.options.stability !== false,
      showWarSupport: ctx.options.warSupport !== false,
      zoom: ctx.options.scale === 'zoom',
      rawMarkers: extra ?? [],
    };
  },
};
