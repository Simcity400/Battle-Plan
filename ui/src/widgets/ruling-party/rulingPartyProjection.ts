import { categoricalPalette, DEFAULT_DARK_SURFACE } from '../../charts/palette.ts';
import { contentTitle } from '../../dashboard/contentTitles.ts';
import { countryFromKey, countryKey } from '../../dashboard/countryKey.ts';
import { rowAmount } from '../../dashboard/follow/painters.ts';
import { followedRow, keyId } from '../../dashboard/keyedSeriesData.ts';
import {
  FutureDataError,
  type BoundedReadView,
  type BoundedSeries,
  type FamilyPlanContext,
  type SourcePlan,
} from '../../families/contract.ts';
import type { TooltipListItem } from '../../families/host/tooltips.ts';
import {
  registerTimeChart,
  type TimeChartProjection,
  type TimeChartWidgetDeclaration,
} from '../../families/time-chart/declaration.ts';
import type { ChartGap, ChartHoverDetail, ChartMarker, ChartMarkerItem } from '../../families/time-chart/model.ts';
import {
  branchGapsOf,
  buildBoundedStack,
  seriesHeadValue,
} from '../../families/time-chart/seriesHelpers.ts';
import type { PartyHistoryReply } from '../../generated/dashboard';
import { OPTIONS } from './definition.ts';
import {
  bandKeys,
  bands,
  formatPopularity,
  formatShare,
  leaderAt,
  leaderShown,
  partyBadges,
  partyNameAt,
  POPULARITY_FAMILY,
  POPULARITY_PART_BITS,
  rulingAt,
  rulingFirst,
  validatePartyHistoryReply,
  type Band,
} from './partyData.ts';

const PERCENT_TICKS = [0, 50, 100] as const;

export interface RulingPartyOptions {
  readonly tag: string | null;
  readonly history: PartyHistoryReply | null;
  readonly extent: readonly [number, number];
  readonly gaps: readonly ChartGap[];
  readonly surface: string;
  readonly showChanges: boolean;
  readonly showLeaders: boolean;
}

interface RawKeyedState {
  readonly country?: string | number;
  readonly part?: string;
  readonly hour: number;
  readonly run: { readonly valueRaw: string } | null;
  readonly missing: string | null;
}

function bandHeadValue(
  band: Band,
  seriesMap: ReadonlyMap<string, BoundedSeries> | undefined,
  readView: BoundedReadView,
  decimals: number,
): number | null {
  const answer = readView.answer('popularity');
  if (answer && answer.kind !== 'notRecorded') {
    for (const key of band.keys) {
      const row = followedRow(answer, key, POPULARITY_PART_BITS);
      if (row < 0) continue;
      const val = rowAmount(answer, row, decimals);
      if (val !== undefined && val !== null && Number.isFinite(val)) return val;
    }
  }
  for (const key of band.keys) {
    const partSeries = seriesMap?.get(keyId(key)) ?? null;
    if (!partSeries) continue;
    const val = seriesHeadValue(partSeries, undefined, 0, decimals, 1);
    if (val !== null) return val;
  }
  return null;
}

function hoverBandRaw(
  band: Band,
  states: readonly RawKeyedState[],
  hour: number,
): { readonly raw: string } | { readonly missing: string } | null {
  let missingReason: string | null = null;
  for (const state of states) {
    if (state.hour !== hour) continue;
    const tag = state.country === undefined ? '' : countryKey(state.country as string | number);
    const part = state.part ?? '';
    if (!band.keys.some(k => k.tag === tag && k.part === part)) continue;
    if (state.run) return { raw: state.run.valueRaw };
    if (state.missing) missingReason ??= state.missing;
  }
  return missingReason !== null ? { missing: missingReason } : null;
}

function rulingHoverDetail(
  readView: BoundedReadView,
  history: PartyHistoryReply,
  list: readonly Band[],
): ChartHoverDetail | null {
  let hover;
  try {
    hover = readView.hover<{ readonly states?: readonly RawKeyedState[]; readonly decimals?: number } | readonly RawKeyedState[]>('popularityHover');
  } catch (error) {
    if (error instanceof FutureDataError) return null;
    throw error;
  }
  if (!hover) return null;
  const h = hover.hour;
  const data = hover.data;
  const states: readonly RawKeyedState[] = !data
    ? []
    : Array.isArray(data)
      ? data
      : 'states' in data && Array.isArray(data.states)
        ? data.states
        : [];
  const decimals = data && !Array.isArray(data) && 'decimals' in data && typeof data.decimals === 'number'
    ? data.decimals
    : history.decimals;
  const shown = leaderShown(history, leaderAt(history, h));
  const partyName = partyNameAt(history, h);
  const partyLabel = partyName === undefined ? null : partyName ?? 'Not known';
  const ruling = rulingAt(history, h);
  const items: TooltipListItem[] = [
    {
      label: 'Leader',
      value: shown.name,
      ...(shown.known ? { strong: true } : { tone: 'muted' as const }),
    },
  ];
  if (shown.ideology || partyLabel) {
    items.push({
      label: 'Ideology · party',
      value: [shown.ideology ?? 'Not recorded', partyLabel ?? 'Not recorded'].join(' · '),
    });
  }
  const scored = list.map(band => {
    const read = hoverBandRaw(band, states, h);
    return {
      band,
      read,
      sortVal: read && 'raw' in read ? Number(read.raw) : -Infinity,
    };
  }).sort((a, b) => b.sortVal - a.sortVal);
  scored.forEach(({ band, read }, idx) => {
    const label = band.token === ruling ? `${band.name} (ruling)` : band.name;
    if (read && 'raw' in read) {
      items.push({
        label,
        swatch: band.color,
        value: formatPopularity(read.raw, decimals),
        ...(idx === 0 ? { divider: true } : {}),
      });
    } else {
      items.push({
        label,
        swatch: band.color,
        missingReason: read && 'missing' in read ? read.missing : 'Not recorded',
        tone: 'muted',
        ...(idx === 0 ? { divider: true } : {}),
      });
    }
  });
  return { hour: h, items, replacesSeries: true };
}

function buildRulingMarkers(
  history: PartyHistoryReply,
  ordered: readonly Band[],
  options: RulingPartyOptions,
  hour: number,
): ChartMarker[] {
  if (!options.showChanges && !options.showLeaders) return [];
  const names = new Map(
    history.ideologies.map(ideology => [ideology.token, contentTitle(ideology.name, ideology.token, 'Ideology name unavailable')]),
  );
  for (const band of ordered) names.set(band.token, band.name);
  const nameOf = (token: string) => names.get(token) ?? 'Ideology name unavailable';
  const all = partyBadges(history, nameOf, options.showLeaders);
  const kept = options.showChanges ? all : all.filter(badge => badge.glyph === '♔');
  const byDay = new Map<number, { hour: number; items: ChartMarkerItem[] }>();
  for (const badge of kept) {
    if (badge.hour < options.extent[0] || badge.hour > hour) continue;
    const dayKey = Math.floor(badge.hour / 24);
    let group = byDay.get(dayKey);
    if (!group) {
      group = { hour: badge.hour, items: [] };
      byDay.set(dayKey, group);
    }
    group.items.push({ title: badge.label });
  }
  return [...byDay.entries()]
    .sort((a, b) => a[1].hour - b[1].hour)
    .map(([dayKey, group]) => ({
      id: `rp-m:${dayKey}`,
      hour: group.hour,
      title: group.items[0]!.title,
      items: group.items,
    }));
}

export const rulingPartyProjection: TimeChartProjection<RulingPartyOptions> = registerTimeChart({
  kind: 'ruling-party',
  family: 'time-chart',
  variant: 'stacked-area',
  joinMode: 'join',
  measureTitle: 'Ruling party',
  project(readView: BoundedReadView, options: RulingPartyOptions) {
    const hour = readView.hour;
    const history = options.history;
    const tag = options.tag;
    if (!history || !tag) {
      return {
        kind: 'ruling-party',
        measureTitle: 'Ruling party',
        mode: 'stack',
        hour,
        extent: options.extent,
        yAxis: { format: 'percent', fixed: [0, 100], ticks: PERCENT_TICKS },
        series: [],
        empty: null,
      };
    }

    const ordered = bands(history, tag, categoricalPalette(options.surface));
    const ruling = rulingAt(history, hour);
    const list = rulingFirst(ordered, ruling);
    const leader = leaderShown(history, leaderAt(history, hour));
    const partyName = partyNameAt(history, hour);
    const leaderParts = [leader.name, leader.ideology, partyName ?? null].filter((part): part is string => !!part);
    const note = leaderParts.length > 0 ? leaderParts.join(' · ') : null;

    if (!list.length) {
      return {
        kind: 'ruling-party',
        measureTitle: 'Ruling party',
        mode: 'stack',
        hour,
        extent: options.extent,
        yAxis: { format: 'percent', fixed: [0, 100], ticks: PERCENT_TICKS },
        series: [],
        note,
        warnings: history.warnings,
        empty: {
          message: history.leaders.length ? 'Party popularity is unavailable.' : 'Party data is unavailable.',
          kind: 'unavailable',
        },
      };
    }

    const seriesMap = readView.series('popularity');
    const heads = list.map(band => bandHeadValue(band, seriesMap, readView, history.decimals));
    const stack = buildBoundedStack(
      list.map((band, idx) => {
        const parts = band.keys
          .map(key => seriesMap?.get(keyId(key)))
          .filter((item): item is BoundedSeries => item !== undefined);
        const headValue = heads[idx]!;
        return {
          id: band.token,
          key: band.token,
          name: band.name,
          color: band.color,
          opacity: 0.84,
          parts,
          valueScale: 1,
          headValue,
          endLabel: headValue === null
            ? null
            : {
                colorBar: band.color,
                name: band.name,
                fullName: band.token === ruling ? `${band.name} (ruling)` : band.name,
                value: formatShare(headValue),
              },
        };
      }),
      hour,
      { missingMode: 'zero-absent' },
    );

    const markers = buildRulingMarkers(history, ordered, options, hour);
    const gaps: ChartGap[] = [
      ...options.gaps,
      ...history.unobserved
        .filter(span => span.firstHour <= hour)
        .map(span => ({
          firstHour: span.firstHour,
          lastHour: span.lastHour,
          reason: 'Parties were not recorded',
        })),
    ];

    const auditRows = list.flatMap((band, idx) => {
      const head = heads[idx];
      return head === null || head === undefined
        ? []
        : [{ row: band.token, key: band.token, value: formatShare(head) }];
    });

    return {
      kind: 'ruling-party',
      measureTitle: 'Ruling party',
      mode: 'stack',
      hour,
      extent: options.extent,
      yAxis: {
        format: 'percent',
        fixed: [0, 100],
        ticks: PERCENT_TICKS,
      },
      series: [],
      stack,
      markers: markers.length > 0 ? markers : undefined,
      gaps: gaps.length > 0 ? gaps : undefined,
      note,
      hoverDetail: rulingHoverDetail(readView, history, list),
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
  optionFields: OPTIONS,
  countries: { mode: 'single', choose: 'Choose a country to see its politics.' },
  aux: {
    product: 'party-history',
    params: (track, _options, countryTag) => (
      countryTag ? { branchId: track.branch.id, country: countryFromKey(countryTag) } : null
    ),
    validate: validatePartyHistoryReply,
  },
  sources(ctx: FamilyPlanContext<PartyHistoryReply>): SourcePlan {
    const tag = ctx.countryTag;
    const history = ctx.aux;
    if (!tag || !history) {
      return { owner: `ruling-party:${ctx.track.source.id}:${ctx.branch.id}:${tag ?? ''}`, entries: [] };
    }
    const keys = bandKeys(bands(history, tag, categoricalPalette(DEFAULT_DARK_SURFACE)));
    if (!keys.length) {
      return {
        owner: `ruling-party:${ctx.track.source.id}:${ctx.branch.id}:${tag}:empty`,
        entries: [],
        allowEmptyFollow: true,
      };
    }
    const reads = keys.map(key => ({ country: countryFromKey(key.tag), part: key.part }));
    return {
      owner: `ruling-party:${ctx.track.source.id}:${ctx.branch.id}:${tag}:${keys.map(keyId).join(',')}`,
      entries: [
        {
          name: 'popularity',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'pixel-binned',
            coverage: { mode: 'keyed', keys: keys.map(keyId) },
            family: POPULARITY_FAMILY,
            tags: [tag],
            keys,
            partBits: POPULARITY_PART_BITS,
            headQuery: {
              product: 'series-state',
              params: {
                branchId: ctx.branch.id,
                family: POPULARITY_FAMILY,
                reads: [{ country: countryFromKey(tag) }],
              },
            },
            exactProduct: 'series-state',
          },
        },
        {
          name: 'popularityHover',
          source: ctx.track.source,
          branch: ctx.branch.id,
          offset: ctx.track.axisOffset,
          read: {
            shape: 'exact-hover',
            product: 'series-state',
            params: {
              branchId: ctx.branch.id,
              family: POPULARITY_FAMILY,
              reads,
            },
          },
        },
      ],
    };
  },
  projection: rulingPartyProjection,
  projectionOptions(ctx, { extent, surface }) {
    return {
      tag: ctx.countryTag,
      history: ctx.aux ?? null,
      extent,
      gaps: branchGapsOf(ctx.branch),
      surface,
      showChanges: ctx.options.changes !== false,
      showLeaders: ctx.options.leaders !== false,
    };
  },
};
