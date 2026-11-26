import { measureColour } from '../../charts/palette.ts';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import {
  displayedHour,
  FutureDataError,
  type BoundedReadView,
  type FamilyPlanContext,
  type SourcePlan,
} from '../../families/contract.ts';
import type { HolderRun, HourRanges } from '../../families/holders.ts';
import { measureArtHandle } from '../../families/time-chart/measureArt.ts';
import { lineChartSettings } from '../../families/time-chart/settings.ts';
import { type TimeChartProjection, type TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import type {
  ChartArtSlot,
  ChartGap,
  ChartHoverDetail,
  ChartMarker,
  ChartMarkerItem,
  TimeChartSeriesModel,
} from '../../families/time-chart/model.ts';
import { formatRational, formatRecordedPercent, notRecordedLabel } from '../../families/valueFormat.ts';
import type { StabilityBreakdownReply, StabilityMarker, StabilityMarkersReply } from '../../generated/dashboard';
import {
  branchGapsOf,
  cardItems,
  countrySeriesLooks,
  memberAt,
  missingSpansOf,
  rankEntry,
  rankedOrder, unnamedLooks,
  auditReading,
  headReading,
  headValueOf,
  type HeadReading,
  type KnownHeadReading,
  withinMapOf,
} from '../../families/time-chart/seriesHelpers.ts';
import { breakdownRows, linesPerMeasure, type Measure } from './stabilityData.ts';

export interface StabilityOptions {
  readonly tags: readonly string[];
  readonly holders?: Readonly<Record<string, readonly HolderRun[]>> | null;
  readonly membership?: Readonly<Record<string, HourRanges>> | null;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
  readonly showStability: boolean;
  readonly showWarSupport: boolean;
  readonly zoom: boolean;
  readonly windowHours?: number | null;
}


const PERCENT_SCALE = { numerator: 100n } as const;

function formatStabilityReading(reading: KnownHeadReading): string {
  if (reading.raw !== undefined && reading.denominator !== undefined) {
    return formatRational(reading.raw, reading.denominator, { maxDecimals: 5, percent: true });
  }
  return formatRecordedPercent(reading.value);
}

const FACTION_VERBS = { founded: 'Founded', joined: 'Joined', left: 'Left' } as const;

function resolveMarkerTitle(m: StabilityMarker, readView: BoundedReadView, limitHour: number): string {
  const at = displayedHour(Math.min(m.hour, limitHour));
  if (m.other && (m.verb === 'began' || m.verb === 'capitulated')) {
    const who = readView.who('country', m.other, at);
    if (who.name.status !== 'ready') return m.title;
    return m.verb === 'began' ? `War with ${who.name.value} began` : `${who.name.value} capitulated`;
  }
  if (m.faction && (m.verb === 'founded' || m.verb === 'joined' || m.verb === 'left')) {
    const who = readView.who('faction', m.faction, at);
    return who.name.status === 'ready' ? `${FACTION_VERBS[m.verb]} ${who.name.value}` : m.title;
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
    const mapped = cardItems(rows, true);
    if (items.length > 0 && mapped[0]) mapped[0] = { ...mapped[0], divider: true };
    items.push(...mapped);
  }
  return { hour: hover.hour, items };
}

function markerPriority(m: StabilityMarker): number {
  if (m.alert) return 3;
  if (m.kind === 'focus' && m.sprite) return 2;
  if (m.kind === 'idea' && m.sprite) return 1;
  return 0;
}

interface MarkerHour {
  readonly hour: number;
  readonly markers: readonly StabilityMarker[];
}

const markerHourCache = new WeakMap<readonly StabilityMarker[], readonly MarkerHour[]>();

function markerHours(raw: readonly StabilityMarker[]): readonly MarkerHour[] {
  const held = markerHourCache.get(raw);
  if (held) return held;
  const byHour = new Map<number, StabilityMarker[]>();
  for (const m of raw) {
    const list = byHour.get(m.hour);
    if (list) list.push(m);
    else byHour.set(m.hour, [m]);
  }
  const hours = [...byHour].map(([hour, list]) => {
    list.sort((a, b) => markerPriority(b) - markerPriority(a));
    return { hour, markers: list };
  }).sort((a, b) => a.hour - b.hour);
  markerHourCache.set(raw, hours);
  return hours;
}

export const stabilityProjection: TimeChartProjection<StabilityOptions> = registerFamilyProjection({
  kind: 'stability',
  family: 'time-chart',
  variant: 'line',
  joinMode: 'step',
  measureTitle: 'Stability & War support',
  project(readView: BoundedReadView, options: StabilityOptions) {
    const hour = readView.hour;
    const tags = options.tags;
    const single = tags.length === 1;
    const stabMap = options.showStability ? readView.series('stability') : undefined;
    const warMap = options.showWarSupport ? readView.series('warSupport') : undefined;

    const stabByTag = new Map<string, HeadReading | null>();
    const warByTag = new Map<string, HeadReading | null>();
    const readings: [string, HeadReading | null][] = [];
    for (let i = 0; i < tags.length; i++) {
      const tag = tags[i]!;
      const member = memberAt(options.membership, tag, hour);
      if (!member) continue;
      if (options.showStability) {
        const reading = headReading(readView, 'stability', { tag }, 5, PERCENT_SCALE);
        readings.push([single ? 'stability' : `${tag}:stability`, reading]);
        stabByTag.set(tag, reading);
      }
      if (options.showWarSupport) {
        const reading = headReading(readView, 'warSupport', { tag }, 5, PERCENT_SCALE);
        readings.push([single ? 'warSupport' : `${tag}:warSupport`, reading]);
        warByTag.set(tag, reading);
      }
    }

    const present = tags.filter(tag => memberAt(options.membership, tag, hour));
    const presentSet = new Set(present);
    const ranking = rankedOrder(readView, present);
    const ordered = [...ranking.order, ...tags.filter(tag => !presentSet.has(tag))];

    const looks = unnamedLooks(countrySeriesLooks(ordered, readView, {
      holders: options.holders,
      from: options.extent[0],
      surface: options.surface,
      withFlag: !single,
      singleColor: measureColour('stability', options.surface),
    }), ranking.unnamed, hour);

    const stabIcon: ChartArtSlot | undefined = single && options.showStability
      ? { art: readView.art(measureArtHandle('stability')), label: 'Stability' }
      : undefined;
    const warIcon: ChartArtSlot | undefined = single && options.showWarSupport
      ? { art: readView.art(measureArtHandle('warSupport')), label: 'War support' }
      : undefined;

    const series: TimeChartSeriesModel[] = [];
    for (let rank = 0; rank < ordered.length; rank++) {
      const tag = ordered[rank]!;
      const isTail = !single && rank >= 8;
      const { name, flag, color: countryColor, identities } = looks.get(tag)!;
      const named = (suffix: string) => (single ? {} : { identities: identities.map(run => ({ ...run, name: `${run.name} · ${suffix}` })) });

      if (options.showStability) {
        const bounded = stabMap?.get(tag) ?? null;
        const stabReading = stabByTag.get(tag) ?? null;
        const headValue = headValueOf(stabReading);
        series.push({
          id: single ? 'stability' : `${tag}:stability`,
          key: single ? 'stability' : `${tag}:stability`,
          name: single ? 'Stability' : `${name} · Stability`,
          color: single ? measureColour('stability', options.surface) : countryColor,
          role: isTail ? 'tail' : 'primary',
          step: true,
          area: false,
          bounded,
          valueScale: 100,
          headValue,
          headHour: hour,
          flag,
          ...named('Stability'),
          icon: single ? stabIcon : undefined,
          missingSpans: missingSpansOf(bounded),
          endLabel: isTail || !stabReading || !('value' in stabReading)
            ? null
            : {
                ...(single ? { icon: stabIcon } : { flag }),
                name: single ? 'Stability' : `${name} · Stability`,
                fullName: single ? 'Stability' : `${name} Stability`,
                value: formatStabilityReading(stabReading),
              },
        });
      }

      if (options.showWarSupport) {
        const bounded = warMap?.get(tag) ?? null;
        const warReading = warByTag.get(tag) ?? null;
        const headValue = headValueOf(warReading);
        series.push({
          id: single ? 'warSupport' : `${tag}:warSupport`,
          key: single ? 'warSupport' : `${tag}:warSupport`,
          name: single ? 'War support' : `${name} · War support`,
          color: single ? measureColour('warSupport', options.surface) : countryColor,
          role: isTail ? 'tail' : 'primary',
          step: true,
          dash: single ? undefined : [5, 4],
          area: false,
          bounded,
          valueScale: 100,
          headValue,
          headHour: hour,
          flag,
          ...named('War support'),
          icon: single ? warIcon : undefined,
          missingSpans: missingSpansOf(bounded),
          endLabel: isTail || !warReading || !('value' in warReading)
            ? null
            : {
                ...(single ? { icon: warIcon } : { flag }),
                name: single ? 'War support' : `${name} · War support`,
                fullName: single ? 'War support' : `${name} War support`,
                value: formatStabilityReading(warReading),
              },
        });
      }
    }

    const markers: ChartMarker[] = [];
    const markersReply = single ? readView.panel<StabilityMarkersReply>('markers') : undefined;
    const recorded = markersReply?.markers ?? [];
    const warnings = [...ranking.warnings, ...(markersReply?.warnings ?? [])];
    if (recorded.length > 0) {
      for (const day of markerHours(recorded)) {
        if (day.hour > hour) break;
        if (day.hour < options.extent[0]) continue;
        const iconSlot = (m: StabilityMarker): ChartArtSlot | undefined => {
          if (m.alert) return { art: readView.art(measureArtHandle('warAlert')), label: 'War' };
          if (m.sprite) return { art: readView.art({ kind: 'sprite', key: m.sprite, frame: 0 }), label: m.title };
          return undefined;
        };
        const items: ChartMarkerItem[] = day.markers.map(m => ({
          title: resolveMarkerTitle(m, readView, hour),
          icon: iconSlot(m),
        }));
        markers.push({
          id: `stab-m:${day.hour}:${day.markers[0]!.id}`,
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
      ...(options.windowHours ? { view: { x: [Math.max(options.extent[0], hour - options.windowHours), hour] as const } } : {}),
      yAxis: {
        format: 'percent',
        fixed: options.zoom ? null : [0, 100],
      },
      series,
      markers: markers.length > 0 ? markers : undefined,
      ...(warnings.length > 0 ? { warnings } : {}),
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
      auditRows: readings.map(([row, reading]) => auditReading(row, reading, formatStabilityReading)),
    };
  },
});

export const stabilityDeclaration: TimeChartWidgetDeclaration<StabilityOptions> = {
  family: 'time-chart',
  kind: 'stability',
  label: 'Stability & War support',
  subject: 'stability and war support',
  emptyCampaign: 'Load a campaign to see stability and war support.',
  icon: 'chart-line',
  countries: { mode: 'multi', allowSlots: true },
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
          headQuery: tags.length
            ? { product: 'series-state', coverage: 'row', params: { branchId: ctx.branch.id, family, reads: countries.map(country => ({ country })) } }
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
          headQuery: tags.length
            ? { product: 'series-state', coverage: 'row', params: { branchId: ctx.branch.id, family, reads: countries.map(country => ({ country })) } }
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
      const marks = { focus: ctx.options.markFocus !== false, government: ctx.options.markGovernment !== false, wars: ctx.options.markWars !== false, factions: ctx.options.markFactions !== false };
      if (marks.focus || marks.government || marks.wars || marks.factions) {
        const params = { branchId: ctx.branch.id, country: countries[0]!, ...marks };
        entries.push({
          name: 'markers',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'point',
            channel: 'panel',
            coverage: { mode: 'keyed', keys: tags },
            product: 'stability-markers',
            followQuery: { panel: 'stability-markers', ...params },
            params,
          },
        });
      }
    }
    const rank = rankEntry(ctx, { kind: 'country-stability', measure: showStab ? 'stability' : 'warSupport' }, tags);
    if (rank && (showStab || showWar)) entries.push(rank);
    return {
      owner: `stability:${ctx.track.source.id}:${ctx.branch.id}:${tags.join(',')}:${showStab ? 1 : 0}:${showWar ? 1 : 0}:${['markFocus', 'markGovernment', 'markWars', 'markFactions'].map(key => ctx.options[key] !== false ? 1 : 0).join('')}`,
      entries,
    };
  },
  projection: stabilityProjection,
  projectionOptions(ctx, { extent, surface }) {
    return {
      tags: ctx.countries.tags,
      holders: ctx.countries.holders,
      membership: ctx.countries.membership,
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
      showStability: ctx.options.stability !== false,
      showWarSupport: ctx.options.warSupport !== false,
      zoom: ctx.options.scale === 'zoom',
      windowHours: lineChartSettings(ctx.options, ctx.following).windowHours,
    };
  },
};
