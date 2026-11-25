import { auditRow } from '../../families/auditRow.ts';
import { categoricalPalette, DEFAULT_DARK_SURFACE } from '../../charts/palette.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { keyId, type SeriesKey } from '../../dashboard/keyedSeriesData.ts';
import {
  FutureDataError,
  type BoundedReadView,
  type FamilyPlanContext,
  type SourcePlan,
} from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import { type TimeChartProjection, type TimeChartWidgetDeclaration } from '../../families/time-chart/declaration.ts';
import { registerFamilyProjection } from '../../families/registry.ts';
import type { ChartGap, ChartHoverDetail, ChartMarker, ChartMarkerItem } from '../../families/time-chart/model.ts';
import {
  branchGapsOf,
  buildBoundedStack,
  headReading,
  type HeadReading,
  type KnownHeadReading,
  type StackPart,
} from '../../families/time-chart/seriesHelpers.ts';
import { formatShare, formatShareRatio } from '../../families/valueFormat.ts';
import type { PartyHistoryReply } from '../../generated/dashboard';
import {
  bandKeys,
  bands,
  leaderAt,
  leaderShown,
  ownedPartAt,
  partyBadges,
  partyNameAt,
  POPULARITY_FAMILY,
  POPULARITY_PART_BITS,
  rulingAt,
  validatePartyHistoryReply,
  type Band,
} from './partyData.ts';

const PERCENT_TICKS = [0, 50, 100] as const;
const Y_AXIS = { format: 'share', fixed: [0, 100], ticks: PERCENT_TICKS } as const;

export interface RulingPartyOptions {
  readonly subject: string | null;
  readonly history: PartyHistoryReply | null;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
  readonly showChanges: boolean;
  readonly showLeaders: boolean;
}

interface HoverState {
  readonly country?: string | number;
  readonly part?: string;
  readonly hour: number;
  readonly run: { readonly valueRaw: string } | null;
  readonly missing: string | null;
}

const partKey = (subject: string, part: number): SeriesKey => ({ tag: subject, part: String(part) });

function stackParts(band: Band, subject: string, readView: BoundedReadView): StackPart[] {
  const byPart = new Map<number, [number, number][]>();
  for (const owned of band.owned) {
    const within = byPart.get(owned.part);
    if (within) within.push([owned.firstHour, owned.lastHour]);
    else byPart.set(owned.part, [[owned.firstHour, owned.lastHour]]);
  }
  const series = readView.series('popularity');
  return [...byPart].map(([part, within]) => {
    const bounded = series?.get(keyId(partKey(subject, part)));
    return bounded ? { series: bounded, within } : { within };
  });
}

function formatPopularityReading(reading: KnownHeadReading): string {
  if (reading.raw !== undefined && reading.denominator !== undefined) {
    return formatShareRatio(reading.raw, reading.denominator * 100n);
  }
  return formatShare(reading.value / 100);
}

function bandHead(band: Band, subject: string, readView: BoundedReadView, hour: number, decimals: number): HeadReading | null | undefined {
  const part = ownedPartAt(band, hour);
  if (part === null) return undefined;
  return headReading(readView, 'popularity', { tag: subject, part: String(part), partBits: POPULARITY_PART_BITS }, decimals);
}

function rulingHoverDetail(readView: BoundedReadView, history: PartyHistoryReply, list: readonly Band[], subject: string): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<{ readonly states?: readonly HoverState[]; readonly decimals?: number }>('popularityHover');
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover?.data) return null;
  const at = hover.hour;
  const states = Array.isArray(hover.data.states) ? hover.data.states : [];
  const decimals = typeof hover.data.decimals === 'number' ? hover.data.decimals : history.decimals;
  const shown = leaderShown(history, leaderAt(history, at));
  const partyName = partyNameAt(history, at);
  const partyLabel = partyName === undefined ? null : partyName ?? 'Not known';
  const ruling = rulingAt(history, at);
  const items: TooltipListItem[] = [
    { label: 'Leader', value: shown.name, ...(shown.known ? { strong: true } : { tone: 'muted' as const }) },
  ];
  if (shown.ideology || partyLabel) {
    items.push({ label: 'Ideology · party', value: [shown.ideology ?? 'Not recorded', partyLabel ?? 'Not recorded'].join(' · ') });
  }
  let first = true;
  for (const band of list) {
    const part = ownedPartAt(band, at);
    if (part === null) continue;
    const state = states.find(item => item.hour === at && item.part === String(part)
      && item.country !== undefined && countryKey(item.country) === subject);
    const label = band.token === ruling ? `${band.name} (ruling)` : band.name;
    const divider = first ? { divider: true } : {};
    first = false;
    items.push(state?.run
      ? { label, swatch: band.color, value: formatShareRatio(BigInt(state.run.valueRaw), 100n * (10n ** BigInt(decimals))), ...divider }
      : { label, swatch: band.color, missingReason: state?.missing ?? 'Not recorded', tone: 'muted', ...divider });
  }
  return { hour: at, items, replacesSeries: true };
}

function buildRulingMarkers(history: PartyHistoryReply, list: readonly Band[], options: RulingPartyOptions, hour: number): ChartMarker[] {
  if (!options.showChanges && !options.showLeaders) return [];
  const names = new Map(history.ideologies.map(ideology => [ideology.token, contentTitle(ideology.name, ideology.token, 'Ideology name unavailable')]));
  for (const band of list) names.set(band.token, band.name);
  const nameOf = (token: string) => names.get(token) ?? 'Ideology name unavailable';
  const all = partyBadges(history, nameOf, hour, options.showLeaders);
  const kept = options.showChanges ? all : all.filter(badge => badge.glyph === '♔');
  const byHour = new Map<number, ChartMarkerItem[]>();
  for (const badge of kept) {
    if (badge.hour < options.extent[0] || badge.hour > hour) continue;
    const items = byHour.get(badge.hour);
    if (items) items.push({ title: badge.label });
    else byHour.set(badge.hour, [{ title: badge.label }]);
  }
  return [...byHour].map(([at, items]) => ({ id: `rp-m:${at}`, hour: at, title: items[0]!.title, items }));
}

export const rulingPartyProjection: TimeChartProjection<RulingPartyOptions> = registerFamilyProjection({
  kind: 'ruling-party',
  family: 'time-chart',
  variant: 'stacked-area',
  joinMode: 'join',
  measureTitle: 'Ruling party',
  project(readView: BoundedReadView, options: RulingPartyOptions) {
    const hour = readView.hour;
    const { history, subject } = options;
    const base = { kind: 'ruling-party', measureTitle: 'Ruling party', mode: 'stack', hour, extent: options.extent, yAxis: Y_AXIS, series: [] } as const;
    if (!history || !subject) return { ...base, empty: null };

    const list = bands(history, categoricalPalette(options.surface));
    const ruling = rulingAt(history, hour);
    const leader = leaderShown(history, leaderAt(history, hour));
    const partyName = partyNameAt(history, hour);
    const leaderParts = [leader.name, leader.ideology, partyName ?? null].filter((part): part is string => !!part);
    const summaryText = leaderParts.length > 0 ? leaderParts.join(' · ') : null;

    if (!list.length) {
      return {
        ...base,
        summaryText,
        warnings: history.warnings,
        empty: { message: history.leaders.length ? 'Party popularity is unavailable.' : 'Party data is unavailable.', kind: 'unavailable' },
      };
    }

    const heads = list.map(band => bandHead(band, subject, readView, hour, history.decimals));
    const stack = buildBoundedStack(list.map((band, idx) => {
      const head = heads[idx];
      return {
        id: band.token,
        key: band.token,
        name: band.name,
        color: band.color,
        opacity: 0.84,
        parts: stackParts(band, subject, readView),
        head: head ?? null,
        endLabel: !head || !('value' in head)
          ? null
          : { colorBar: band.color, name: band.name, fullName: band.token === ruling ? `${band.name} (ruling)` : band.name, value: formatPopularityReading(head) },
      };
    }), hour);

    const markers = buildRulingMarkers(history, list, options, hour);
    const gaps: ChartGap[] = [
      ...options.gaps,
      ...history.unobserved
        .filter(span => span.firstHour <= hour)
        .map(span => ({ firstHour: span.firstHour, lastHour: span.lastHour, reason: 'Parties were not recorded' })),
    ];
    const auditRows = list.flatMap((band, idx) => {
      const head = heads[idx];
      if (head === undefined) return [];
      if (head === null) return [auditRow(band.token, 'Not read')];
      return [auditRow(band.token, 'value' in head ? formatPopularityReading(head) : head.missing)];
    });

    return {
      ...base,
      stack,
      markers: markers.length > 0 ? markers : undefined,
      gaps: gaps.length > 0 ? gaps : undefined,
      summaryText,
      hoverDetail: rulingHoverDetail(readView, history, list, subject),
      warnings: history.warnings,
      auditRows,
    };
  },
});

export const rulingPartyDeclaration: TimeChartWidgetDeclaration<RulingPartyOptions, PartyHistoryReply> = {
  family: 'time-chart',
  kind: 'ruling-party',
  label: 'Ruling party',
  subject: 'ruling party',
  emptyCampaign: 'Load a campaign and choose a country to see its politics.',
  icon: 'palette',
  countries: { mode: 'single', choose: 'Choose a country to see its politics.' },
  aux: {
    product: 'party-history',
    params: (track, _options, _country, subject) => (
      subject ? { branchId: track.branch.id, country: countryFromKey(subject) } : null
    ),
    validate: validatePartyHistoryReply,
  },
  sources(ctx: FamilyPlanContext<PartyHistoryReply>): SourcePlan {
    const subject = ctx.countrySubject;
    const history = ctx.aux;
    const owner = `ruling-party:${ctx.track.source.id}:${ctx.branch.id}:${subject ?? ''}`;
    if (!subject || !history) return { owner, entries: [] };
    const keys = bandKeys(bands(history, categoricalPalette(DEFAULT_DARK_SURFACE)), subject);
    if (!keys.length) return { owner: `${owner}:empty`, entries: [], allowEmptyFollow: true };
    const country = countryFromKey(subject);
    const reads = keys.map(key => ({ country, part: key.part }));
    const common = { source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset };
    return {
      owner: `${owner}:${keys.map(keyId).join(',')}`,
      entries: [
        {
          ...common,
          name: 'popularity',
          read: {
            shape: 'pixel-binned',
            coverage: { mode: 'keyed', keys: keys.map(keyId) },
            family: POPULARITY_FAMILY,
            tags: [subject],
            keys,
            partBits: POPULARITY_PART_BITS,
            headQuery: { product: 'series-state', params: { branchId: ctx.branch.id, family: POPULARITY_FAMILY, reads: [{ country }] } },
            params: { branchId: ctx.branch.id, family: POPULARITY_FAMILY, reads },
            exactProduct: 'series-state',
          },
        },
        {
          ...common,
          name: 'popularityHover',
          read: { shape: 'exact-hover', product: 'series-state', params: { branchId: ctx.branch.id, family: POPULARITY_FAMILY, reads } },
        },
      ],
    };
  },
  projection: rulingPartyProjection,
  projectionOptions(ctx, { extent, surface }) {
    return {
      subject: ctx.countrySubject,
      history: ctx.aux ?? null,
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
      showChanges: ctx.options.changes !== false,
      showLeaders: ctx.options.leaders !== false,
    };
  },
};
