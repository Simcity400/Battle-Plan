import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import type { IconName } from '../../components/Icons';
import { validateProductReply, type ProductRequest } from '../../dashboard/client.ts';
import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import { useContentRevision } from '../../dashboard/contentRevision.ts';
import { inFrame } from '../../dashboard/frameUpdates.ts';
import { countryMatch, countryMatchChangeHours, groupUnobserved, membershipUnrecordedAt, noMembersAt, rankedCandidates, resolveCountries, worldFilter } from '../../dashboard/groups';
import { resolveWindow, type WidgetSpec } from '../../dashboard/model';
import { resolveOptions, type OptionField, type OptionValue } from '../../dashboard/options';
import { prepareGate, preparing } from '../../dashboard/prepare.ts';
import { loadProduct, peekProduct, productKey, useProduct } from '../../dashboard/products';
import { asFailure, type Failure } from '../../dashboard/queryError.ts';
import { useCountryNameAt } from '../../dashboard/reachedProduct';
import { useGate } from '../../dashboard/regionGate';
import { axisAt, coverageSpan, lastRecordedHour, SPEC } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import { keyId, keyedTileParams, keyedTileReply, type SeriesKey } from '../../dashboard/keyedSeriesData.ts';
import type { TileReply } from '../../dashboard/tiles/tile.ts';
import { TileView, tileCache, type TiledLine, type TileViewport } from '../../dashboard/tiles/view.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { countRead, measuredWidget } from '../../dashboard/widgetPerf.ts';
import { useRegion, useRegionCoverage, watchBoundary } from '../../dashboard/widgetTime';
import type { CountryFlagChoice, HourSpan } from '../../generated/dashboard';
import { perfLog } from '../../shell/perf.ts';
import type { FamilyStaticProof } from './adapter.ts';
import { sharedDomArtCache } from '../art/domCache.ts';
import { createDomArtProvider } from '../art/provider.ts';
import { boundedUnrecorded, type UnrecordedLine } from '../boundedView.ts';
import {
  buildTailPageRequest,
  compileFollowPlan,
  currentHovers,
  FamilySourceBroker,
  hoverAsks,
  type HeldHover,
  planSignature,
  readPlanExact,
  stabilizeCanvasDimension,
  tailPageOffsets,
} from '../broker.ts';
import type { HolderRun } from '../holders.ts';
import { overlapsHourRanges } from '../holders.ts';
import { identityBrokerFor } from '../identity/broker.ts';
import { createBrokerIdentityProvider, parseEntityKey } from '../identity/provider.ts';
import {
  countryTagKey,
  flagChoiceKey,
  useIdentities,
  useIdentityScope,
  useSlotIndex,
  type IdentityAsk,
  type IdentitySnapshot,
} from '../identity/react.ts';
import type { CountryLooksMap } from '../looks.ts';
import type {
  ArtProvider,
  AuxProductSpec,
  BoundedReadView,
  CompiledFollowSubscription,
  CountryPolicy,
  DeclaredCountryFilter,
  DeclaredCountryState,
  EntityKind,
  FamilyBodyStatus,
  FamilyPlanContext,
  FamilyViewportRequest,
  IdentityProvider,
  SourceEntry,
  SourcePlan,
} from '../plan.ts';
import {
  ReadTarget,
  showBundle,
  unitEntries,
  type PinnedAsk,
  type ShownBundle,
  type TargetView,
} from '../readUnits.ts';
import {
  buildCountryLooksFromIdentity,
  effectiveCountryUnobserved,
  extractFactionData,
  freezeGate,
  selectionUnobservedReason,
  trackGate,
  type FrozenGate,
} from '../sources.ts';
import {
  DEFAULT_TAIL_PAGE_SIZE,
  validateGenericTailReply,
  type DatedLogRow,
  type PagedRowsSnapshot,
} from '../table/rowSource.ts';
import type { FamilyRowWindow } from './adapter.ts';

const NO_TAGS: readonly string[] = [];
const NO_LOOKS: CountryLooksMap = {};
const EVERY_COUNTRY: DeclaredCountryFilter = { passing: null, unrecorded: [], bounds: [] };
const filterCache = new WeakMap<object, Map<string, DeclaredCountryFilter>>();

function buildFilterTimeline(
  tags: readonly string[],
  matches: (tag: string, hour: number) => boolean,
  unrecordedAt: (hour: number) => string | null,
  changes: readonly number[],
  first: number,
  last: number,
): DeclaredCountryFilter {
  const bounds = [...new Set([first, ...changes.filter(at => at > first && at <= last), last + 1])].sort((a, b) => a - b);
  const passing: Record<string, [number, number][]> = {};
  const unrecorded: [number, number, string][] = [];
  for (let at = 0; at + 1 < bounds.length; at++) {
    const from = bounds[at]!, to = bounds[at + 1]! - 1;
    for (const tag of tags) if (matches(tag, from)) {
      const ranges = passing[tag] ??= [];
      const previous = ranges.at(-1);
      if (previous && previous[1] === from - 1) previous[1] = to;
      else ranges.push([from, to]);
    }
    const reason = unrecordedAt(from);
    const previous = unrecorded.at(-1);
    if (reason !== null) {
      if (previous && previous[1] === from - 1 && previous[2] === reason) previous[1] = to;
      else unrecorded.push([from, to, reason]);
    }
  }
  return { passing, unrecorded, bounds };
}

const NO_ENTRIES: readonly SourceEntry[] = [];
const NO_VIEW: TargetView = { kind: 'none', failure: null, bundle: null, held: false };
const NO_PIN: PinnedAsk = {
  timeMode: 'pin',
  branch: { firstHour: 0, lastHour: 0 },
  window: null,
  pinHour: null,
  gate: { recorded: () => null, span: () => null },
};

export interface StaticBundle {
  readonly hour: number;
  readonly span: readonly [number, number] | null;
  readonly follows?: boolean;
  readonly panels: Readonly<Record<string, unknown>>;
  readonly shifts?: Readonly<Record<string, number>>;
}

export type StaticProof = FamilyStaticProof;

export interface BannerFailureInfo {
  readonly failure: Failure;
  readonly retry: () => void;
}

export interface UseFamilyPlanInput<TAux, TExtra> {
  readonly kind: string;
  readonly subject: string;
  readonly emptyCampaign: string;
  readonly icon?: IconName;
  readonly optionFields?: readonly OptionField[];
  readonly countries: CountryPolicy;
  readonly aux?: AuxProductSpec<TAux>;
  readonly sources: (ctx: FamilyPlanContext<TAux>, view: FamilyViewportRequest) => SourcePlan;
  readonly captureExtra?: (ctx: FamilyPlanContext<TAux>, countryLooks: CountryLooksMap | undefined) => TExtra;
  readonly widget: WidgetSpec;
  readonly runtime: WidgetRuntime;
  readonly active: boolean;
  readonly hoverHour?: number | null;
  readonly setOption?: (key: string, value: OptionValue) => void;
  readonly viewport?: { readonly width?: number; readonly height?: number };
  readonly viewportRef?: { readonly current: HTMLElement | null };
  readonly viewportDeadbandPx?: number;
}

export interface UseFamilyPlanResult<TAux, TExtra> {
  readonly track: FamilyPlanContext<TAux>['track'] | undefined;
  readonly options: Readonly<Record<string, OptionValue>>;
  readonly following: boolean;
  readonly owner: string;
  readonly ctx: FamilyPlanContext<TAux> | null;
  readonly displayedCtx: FamilyPlanContext<TAux> | null;
  readonly displayedExtra: TExtra | undefined;
  readonly displayedCountryLooks: CountryLooksMap | undefined;
  readonly plan: SourcePlan | null;
  readonly subscriptions: readonly CompiledFollowSubscription[];
  readonly allowEmptyFollow: boolean;
  readonly followRefused: string | null;
  readonly staticBundle: StaticBundle | null;
  readonly staticProof: StaticProof;
  readonly staticHeld: boolean;
  readonly pagedRows: Readonly<Record<string, PagedRowsSnapshot>> | undefined;
  readonly onRowWindow: (window: FamilyRowWindow, hour: number | null) => void;
  readonly tiledSeries: Readonly<Record<string, ReadonlyMap<string, TiledLine>>>;
  readonly tiledRevision: number;
  readonly onSeriesViewport: (viewport: TileViewport) => void;
  readonly retryTiles: () => void;
  readonly hovers: Readonly<Record<string, { readonly hour: number; readonly data: unknown }>>;
  readonly exactReadView: ((hour: number, rowWindow: FamilyRowWindow | null) => Promise<BoundedReadView | null>) | undefined;
  readonly bodyStatus: FamilyBodyStatus;
  readonly bannerFailure: BannerFailureInfo | null;
  readonly nameFailureDetail: string | undefined;
  readonly groupWarnings: readonly string[];
  readonly moment: number;
  readonly bounds: readonly [number, number] | null;
  readonly pin: number | null;
  readonly offset: number;
  readonly loadingLabel: string;
  readonly identitySnapshot: IdentitySnapshot | null;
  readonly identityProvider: IdentityProvider | undefined;
  readonly artProvider: ArtProvider;
  readonly onIdentityDemands: (demands: readonly (readonly [EntityKind, string, number])[]) => void;
  readonly measureViewport: (width: number, height?: number) => void;
}

export function useFamilyPlan<TAux = unknown, TExtra = undefined>(
  input: UseFamilyPlanInput<TAux, TExtra>,
): UseFamilyPlanResult<TAux, TExtra> {
  const {
    kind,
    subject,
    emptyCampaign,
    optionFields,
    countries: policy,
    aux: auxSpec,
    sources: buildSources,
    captureExtra,
    widget,
    runtime,
    active,
    hoverHour = null,
    setOption,
    viewport,
    viewportRef,
    viewportDeadbandPx = 2,
  } = input;

  const [measuredViewport, setMeasuredViewport] = useState<{ readonly width?: number; readonly height?: number }>({});
  const measureViewport = useCallback((width: number, height?: number) => {
    setMeasuredViewport(prev => {
      const nextWidth = stabilizeCanvasDimension(width, prev.width, viewportDeadbandPx);
      const nextHeight = height === undefined
        ? prev.height
        : stabilizeCanvasDimension(height, prev.height, viewportDeadbandPx);
      if (nextWidth === prev.width && nextHeight === prev.height) return prev;
      return {
        ...(nextWidth !== undefined ? { width: nextWidth } : {}),
        ...(nextHeight !== undefined ? { height: nextHeight } : {}),
      };
    });
  }, [viewportDeadbandPx]);

  useEffect(() => {
    const node = viewportRef?.current;
    if (!node || typeof ResizeObserver !== 'function') return;
    measureViewport(node.clientWidth, node.clientHeight);
    const ro = new ResizeObserver(entries => {
      const rect = entries[0]?.contentRect;
      measureViewport(rect ? rect.width : node.clientWidth, rect ? rect.height : node.clientHeight);
    });
    ro.observe(node);
    return () => { ro.disconnect(); };
  }, [viewportRef, measureViewport]);

  const stableWidth = stabilizeCanvasDimension(viewport?.width, measuredViewport.width, viewportDeadbandPx);
  const stableHeight = stabilizeCanvasDimension(viewport?.height, measuredViewport.height, viewportDeadbandPx);

  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const following = widget.time.mode === 'follow';
  const options = useMemo(
    () => resolveOptions(optionFields ?? [], widget.options),
    [optionFields, widget.options],
  );
  const isSingle = policy.mode === 'single';
  const isFilter = policy.mode === 'filter';
  const isMulti = policy.mode === 'multi';

  const gate = useGate(
    runtime,
    widget,
    SPEC,
    active,
    isSingle ? { country: { stored: widget.options?.country, setOption } } : undefined,
  );
  const coverage = useRegionCoverage(runtime, useRegion(widget));

  const singleCountry = isSingle ? gate.country! : null;
  const countryTag = isSingle && gate.open ? singleCountry!.tag : null;
  const countryLabel = useCountryNameAt(
    runtime.client,
    isSingle ? track : undefined,
    isSingle ? (countryTag ?? singleCountry!.chosen) : null,
    isSingle ? gate.hour : null,
    active && isSingle,
  );
  const countryName = countryLabel.name;
  const nameFailureDetail = isSingle ? countryLabel.nameFailure?.message : undefined;

  const firstHour = track?.branch.firstHour ?? 0;
  const lastHour = track?.branch.lastHour ?? 0;
  const lastRecorded = useMemo(() => lastRecordedHour(track), [track]);
  const effectiveEndHour = lastRecorded ?? lastHour;

  const playbackHour = usePlaybackHour(runtime.clock, false);
  const rawWindow = !following && track ? resolveWindow(widget, track, runtime.tracks, playbackHour) : null;
  const windowFirst = rawWindow?.firstHour ?? null;
  const windowLast = rawWindow?.lastHour ?? null;
  const window = useMemo(
    () => (windowFirst !== null && windowLast !== null ? { firstHour: windowFirst, lastHour: windowLast } : null),
    [windowFirst, windowLast],
  );
  const moment = Math.floor(window?.lastHour ?? effectiveEndHour);
  const pinHour = !following && track ? gate.hourAt(axisAt(playbackHour)) : null;

  const auxEnabled = !!auxSpec && (!auxSpec.when || auxSpec.when(options));
  const auxParams = useMemo(
    () => (auxSpec && auxEnabled && track ? auxSpec.params(track, options, countryTag) : null),
    [auxSpec, auxEnabled, track, options, countryTag],
  );
  const auxReq = useMemo(
    () => (auxSpec && auxParams && track
      ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: auxSpec.product, params: auxParams })
      : null),
    [auxSpec, auxParams, track],
  );
  const auxResult = useProduct<TAux>(
    runtime.client,
    auxReq,
    active && auxEnabled,
    auxSpec?.validate,
    track?.source,
  );

  const needsGroups = isFilter
    || (isMulti && (policy.requireGroupsWhen ? policy.requireGroupsWhen(widget, options) : widget.countries.kind === 'group'));
  const fetchGroups = isFilter || (isMulti && (needsGroups || policy.allTagsForLabels || policy.factions || policy.everyOnTop));
  const groups = useCountryGroups(runtime, track, active && fetchGroups);

  const allTagsText = useMemo(
    () => JSON.stringify((groups.data?.countryIdentities ?? []).map(history => history.tag).sort()),
    [groups.data],
  );
  const allTags = useMemo(() => (allTagsText !== '[]' ? JSON.parse(allTagsText) as string[] : NO_TAGS), [allTagsText]);

  const rawSelected = selectedCountryKeys(widget.countries, track);
  const selectedText = JSON.stringify(rawSelected);
  const selectedFallback = useMemo(() => JSON.parse(selectedText) as string[], [selectedText]);

  const filterMatch = useMemo(
    () => (isFilter ? countryMatch(widget.countries, selectedFallback, groups.data, lastHour) : null),
    [isFilter, widget.countries, selectedFallback, groups.data, lastHour],
  );
  const filter = useMemo<DeclaredCountryFilter>(() => {
    if (!isFilter || widget.countries.kind === 'all' || !filterMatch?.ready || !groups.data || !track) return EVERY_COUNTRY;
    const key = JSON.stringify([widget.countries, selectedFallback, firstHour, lastHour]);
    let held = filterCache.get(groups.data);
    if (!held) filterCache.set(groups.data, held = new Map());
    let made = held.get(key);
    if (!made) {
      made = buildFilterTimeline(
        allTags,
        filterMatch.matches,
        at => membershipUnrecordedAt(widget.countries, groups.data, at),
        countryMatchChangeHours(filterMatch.matches) ?? [],
        firstHour,
        lastHour,
      );
      held.set(key, made);
    }
    return made;
  }, [isFilter, widget.countries, filterMatch, groups.data, track, selectedFallback, allTags, firstHour, lastHour]);

  const resolvedMulti = useMemo(() => {
    if (!isMulti || widget.countries.kind === 'all') return null;
    return resolveCountries(widget.countries, selectedFallback, groups.data, lastHour, Number.MAX_SAFE_INTEGER);
  }, [isMulti, widget.countries, selectedFallback, groups.data, lastHour]);

  const singleSlotId = isMulti && policy.allowSingleSlot && widget.countries.kind === 'selected' && selectedFallback.length === 1
    ? countryFromKey(selectedFallback[0]!)
    : null;
  const anySlot = isMulti && policy.allowSlots && (resolvedMulti?.tags.some(isSlotKey) ?? false);
  const needsSlots = isMulti && (
    typeof singleSlotId === 'number'
    || anySlot
    || (widget.countries.kind !== 'all' && resolvedMulti?.top == null)
  );
  const identityScope = useIdentityScope(runtime.client, track?.source, track?.branch.id, active && !!track);
  const branchBounds = useMemo(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] as const : undefined),
    [track?.branch.firstHour, track?.branch.lastHour],
  );
  const slotState = useSlotIndex(runtime.client, track?.source, identityScope, active && needsSlots, branchBounds);
  const slots = slotState.slots;
  const slotRevision = slotState.revision;

  const effectiveMulti = useMemo(() => {
    if (!resolvedMulti) return null;
    if (typeof singleSlotId !== 'number') return resolvedMulti;
    const runs = [...(slots?.tagRuns(singleSlotId) ?? [])];
    return {
      ...resolvedMulti,
      tags: [...new Set(runs.map(run => run.tag))],
      membership: new Map([['', runs]]),
      known: (from: number, to?: number) => !!slots && resolvedMulti.known(from, to),
    };
  }, [resolvedMulti, singleSlotId, slots, slotRevision]);

  const holders = useMemo<Readonly<Record<string, readonly HolderRun[]>> | null>(() => {
    if (!isMulti || !effectiveMulti || effectiveMulti.top != null || !slots) return null;
    const out: Record<string, readonly HolderRun[]> = {};
    for (const key of effectiveMulti.tags) out[key] = slots.runsForKey(key);
    return out;
  }, [isMulti, effectiveMulti, slots, slotRevision]);

  const unobserved = useMemo(
    () => effectiveCountryUnobserved(widget.countries, effectiveMulti?.unobserved),
    [widget.countries, effectiveMulti?.unobserved],
  );

  const membership = useMemo<Readonly<Record<string, readonly (readonly [number, number])[]>> | null>(() => {
    if (!effectiveMulti?.membership) return null;
    const out: Record<string, [number, number][]> = {};
    for (const [tag, runs] of effectiveMulti.membership) {
      out[tag] = runs.map(run => [run.firstHour, run.lastHour]);
    }
    return out;
  }, [effectiveMulti?.membership]);

  const membershipRuns = useMemo(() => {
    if (!effectiveMulti?.membership) return null;
    return [...effectiveMulti.membership.values()].flat().map(run => ({
      tag: run.tag,
      firstHour: run.firstHour,
      lastHour: run.lastHour,
    }));
  }, [effectiveMulti?.membership]);

  const factionData = useMemo(
    () => extractFactionData(groups.data, isMulti && policy.factions === true),
    [isMulti, policy, groups.data],
  );

  const wantsColours = (isFilter || isMulti) && policy.colours === true;
  const wantsLabels = (isFilter || isMulti) && (policy.labels === true || wantsColours);
  const labelTags = useMemo(() => {
    if (!wantsLabels) return NO_TAGS;
    if (isFilter) return allTags;
    if (isMulti && (policy.allTagsForLabels || (policy.everyOnTop && effectiveMulti?.top != null))) return allTags;
    if (holders) {
      const set = new Set<string>();
      for (const runs of Object.values(holders)) for (const run of runs) set.add(run.tag);
      for (const tag of effectiveMulti?.tags ?? []) if (!isSlotKey(tag)) set.add(tag);
      return [...set].sort();
    }
    return effectiveMulti?.tags ?? allTags;
  }, [wantsLabels, isFilter, isMulti, policy, allTags, effectiveMulti?.top, effectiveMulti?.tags, holders]);

  const [laneDemands, setLaneDemands] = useState<readonly IdentityAsk[]>([]);
  const laneDemandsSigRef = useRef('');
  const onIdentityDemands = useCallback((demands: readonly (readonly [EntityKind, string, number])[]) => {
    const sig = demands.map(([k, key, h]) => `${k}\u0000${key}\u0000${h}`).join('\u0001');
    if (sig === laneDemandsSigRef.current) return;
    laneDemandsSigRef.current = sig;
    setLaneDemands(demands.map(([k, key, h]) => ({ key: parseEntityKey(k, key), hour: h })));
  }, []);

  const wantsIdentity = wantsLabels || laneDemands.length > 0;
  const labelAskHour = pinHour ?? moment;
  const labelAsks = useMemo<readonly IdentityAsk[]>(() => {
    const asks: IdentityAsk[] = [];
    if (wantsLabels && track) {
      const at = Math.max(firstHour, Math.min(lastHour, Math.floor(labelAskHour)));
      for (const tag of labelTags) asks.push({ key: countryTagKey(tag), hour: at });
    }
    for (const demand of laneDemands) asks.push(demand);
    return asks;
  }, [wantsLabels, track, labelTags, firstHour, lastHour, labelAskHour, laneDemands]);
  const identityState = useIdentities(runtime.client, track?.source, wantsIdentity ? identityScope : null, labelAsks, {
    active: active && wantsIdentity,
    needFlags: true,
    flagSize: 'panel',
    branchBounds,
  });
  const identityRevision = identityState.index?.resolveRevision() ?? -1;
  useEffect(() => {
    if (!active || !wantsLabels || !identityScope || !track || !identityState.index?.hasFacts()) return;
    const seen = new Set<string>();
    const choices: CountryFlagChoice[] = [];
    const limit = Math.floor(lastHour);
    for (const tag of labelTags) {
      const key = countryTagKey(tag);
      let cursor = Math.floor(firstHour);
      while (cursor <= limit) {
        const res = identityState.index.resolve(key, cursor);
        if (res.kind === 'known' && res.flagChoice.kind === 'known') {
          const id = flagChoiceKey(res.flagChoice.value);
          if (!seen.has(id)) {
            seen.add(id);
            choices.push(res.flagChoice.value);
          }
        }
        cursor = (res.kind === 'known' || res.kind === 'absent') && Number.isFinite(res.nextBoundary) && res.nextBoundary > cursor
          ? res.nextBoundary
          : limit + 1;
      }
    }
    if (choices.length) identityBrokerFor(runtime.client).requestFlagChoices(identityScope, 'panel', choices, track.source);
  }, [active, wantsLabels, identityScope, runtime.client, track, identityState.index, identityRevision, labelTags, firstHour, lastHour]);

  const builtLooks = useMemo(
    () => (wantsLabels ? buildCountryLooksFromIdentity(identityState.index, labelTags, firstHour, lastHour, identityState.flagFor, wantsColours) : NO_LOOKS),
    [wantsLabels, identityState.index, identityState.version, identityState.flagFor, labelTags, firstHour, lastHour, wantsColours],
  );
  const labelsText = wantsLabels ? JSON.stringify(builtLooks) : '{}';
  const labels = useMemo<CountryLooksMap>(
    () => (wantsLabels ? JSON.parse(labelsText) as CountryLooksMap : NO_LOOKS),
    [wantsLabels, labelsText],
  );

  const [providerOwner] = useState(() => ({}));
  const broker = identityBrokerFor(runtime.client);
  const identityProvider = useMemo<IdentityProvider | undefined>(() => {
    if (!identityScope) return undefined;
    return createBrokerIdentityProvider(broker, identityScope, providerOwner, {
      recording: track?.source,
      branchBounds,
      needFlags: true,
    });
  }, [broker, identityScope, providerOwner, track?.source, branchBounds]);
  useEffect(() => () => {
    if (identityScope) broker.setDemand(providerOwner, identityScope, null);
  }, [broker, identityScope, providerOwner]);

  const artProvider = useMemo<ArtProvider>(
    () => createDomArtProvider(sharedDomArtCache, { broker, scope: identityScope, recording: track?.source }),
    [broker, identityScope, track?.source],
  );
  const identitySnapshot = useMemo<IdentitySnapshot | null>(() => {
    void identityState.version;
    return identityScope ? broker.snapshotOf(identityScope, track?.source, branchBounds) : null;
  }, [broker, identityScope, track?.source, branchBounds, identityState.version]);

  const filterUnobserved = useMemo(
    () => (isFilter && widget.countries.kind === 'group' && groups.data ? groupUnobserved(groups.data, widget.countries.group) : undefined),
    [isFilter, widget.countries, groups.data],
  );

  const countriesState = useMemo<DeclaredCountryState>(() => ({
    all: widget.countries.kind === 'all',
    tags: effectiveMulti?.tags ?? NO_TAGS,
    holders,
    every: allTags,
    top: effectiveMulti?.top ?? null,
    membership,
    membershipRuns,
    unobserved,
    filter,
    labels,
    factions: factionData.runs,
    factionUnobserved: factionData.unobserved,
    factionIncomplete: factionData.incomplete,
    rankedAt: (hour: number) => (effectiveMulti ? rankedCandidates(effectiveMulti, hour, effectiveMulti.tags) : NO_TAGS),
    reason: filterMatch?.reason ?? effectiveMulti?.reason ?? selectionUnobservedReason(widget.countries, filterUnobserved ?? effectiveMulti?.unobserved) ?? null,
    known: (first: number, last: number) => effectiveMulti?.known(first, last) ?? true,
    unobservedIn: (first: number, last: number) => overlapsHourRanges(unobserved, first, last),
  }), [widget.countries, effectiveMulti, filterUnobserved, holders, allTags, membership, membershipRuns, unobserved, filter, labels, factionData, filterMatch?.reason]);

  const owner = useMemo(
    () => JSON.stringify([
      kind,
      track?.source.id ?? '',
      track?.source.revision ?? '',
      track?.branch.id ?? '',
      track?.axisOffset ?? 0,
      widget.time,
      isSingle ? countryTag : widget.countries,
    ]),
    [kind, track?.source.id, track?.source.revision, track?.branch.id, track?.axisOffset, widget.time, isSingle, countryTag, widget.countries],
  );

  const ctx = useMemo<FamilyPlanContext<TAux> | null>(() => {
    if (!track) return null;
    return {
      track,
      branch: track.branch,
      lastRecordedHour: lastRecorded,
      widget,
      options,
      following,
      aux: auxResult.data,
      countryTag,
      countryName,
      countries: countriesState,
      gateSpan: gate.span,
      staticSpan: null,
    };
  }, [track, lastRecorded, widget, options, following, auxResult.data, countryTag, countryName, countriesState, gate.span]);

  const inputsReady = !!ctx
    && (!isSingle || (gate.open && countryTag !== null))
    && (!isFilter || (!!filterMatch?.ready && !filterMatch.reason && !!groups.data))
    && (!auxEnabled || auxResult.data !== undefined)
    && (!isMulti || (!countriesState.reason && (!needsGroups || !!groups.data) && (!needsSlots || !!slots)));

  const rawPlan = useMemo<SourcePlan | null>(() => {
    if (!ctx || !inputsReady) return null;
    return buildSources(ctx, {
      following,
      ...(stableWidth !== undefined ? { width: stableWidth } : {}),
      ...(stableHeight !== undefined ? { height: stableHeight } : {}),
      staticSpan: null,
    });
  }, [ctx, inputsReady, buildSources, following, stableWidth, stableHeight]);

  const stablePlanRef = useRef<{ sig: string; plan: SourcePlan | null }>({ sig: '', plan: null });
  const sig = planSignature(rawPlan);
  if (sig !== stablePlanRef.current.sig) {
    stablePlanRef.current = { sig, plan: rawPlan };
  }
  const plan = stablePlanRef.current.plan;

  const compiledFollow = useMemo(() => {
    if (!following || !active || !plan) {
      return { subscriptions: [] as readonly CompiledFollowSubscription[], refused: null as string | null, unavailable: null as string | null, allowEmptyFollow: false };
    }
    return compileFollowPlan(plan, gate.checkFollow);
  }, [following, active, plan, gate.checkFollow]);

  const pixelBinnedKey = useMemo(() => {
    if (!plan) return '';
    const entries = plan.entries.filter(e => e.read.shape === 'pixel-binned');
    if (!entries.length) return '';
    return JSON.stringify(entries.map(e => {
      const r = e.read as Extract<typeof e.read, { shape: 'pixel-binned' }>;
      return [e.name, e.source.id, e.source.revision, e.branch, r.family, r.tags, r.keys ?? null, r.within ? [...r.within.entries()] : null, r.marks === true, r.top ?? null, lastHour];
    }));
  }, [plan, lastHour]);

  const tileViewsRef = useRef<Map<string, TileView>>(new Map());
  const seriesViewportRef = useRef<TileViewport | null>(null);
  const [tiledState, setTiledState] = useState<{
    readonly key: string;
    readonly series: Readonly<Record<string, ReadonlyMap<string, TiledLine>>>;
    readonly revision: number;
    readonly error: unknown;
    readonly topRefused?: string | null;
  }>({ key: '', series: {}, revision: 0, error: null });

  const planForTilesRef = useRef(plan);
  planForTilesRef.current = plan;
  const gateSpanRef = useRef(gate.span);
  gateSpanRef.current = gate.span;
  const gateRecordedRef = useRef(gate.recorded);
  gateRecordedRef.current = gate.recorded;

  useEffect(() => {
    const currentPlan = planForTilesRef.current;
    if (!active || !pixelBinnedKey || !currentPlan) {
      for (const view of tileViewsRef.current.values()) view.close();
      tileViewsRef.current.clear();
      return;
    }
    const entries = currentPlan.entries.filter(
      (e): e is typeof e & { readonly read: Extract<typeof e.read, { shape: 'pixel-binned' }> } => e.read.shape === 'pixel-binned',
    );
    const views = new Map<string, TileView>();
    let disposed = false;
    let rev = 0;

    const publish = () => {
      if (disposed) return;
      const nextSeries: Record<string, ReadonlyMap<string, TiledLine>> = {};
      let firstError: unknown = null;
      let topRefused: string | null = null;
      for (const [name, tv] of views) {
        if (tv.current) nextSeries[name] = tv.current;
        if (firstError === null && tv.error !== null) firstError = tv.error;
        topRefused ??= tv.top?.refused ?? null;
      }
      rev++;
      const snapshot = { key: pixelBinnedKey, series: nextSeries, revision: rev, error: firstError, topRefused };
      inFrame(() => {
        if (!disposed) setTiledState(snapshot);
      });
    };

    const prepared = preparing(runtime.client);
    for (const entry of entries) {
      const read = entry.read;
      const keyed = read.keys && read.keys.length > 0 ? read.keys : null;
      if (!keyed && !read.tags.length && !read.top) continue;
      const byId = keyed ? new Map<string, SeriesKey>(keyed.map(key => [keyId(key), key])) : null;
      const laneTags = keyed ? keyed.map(keyId) : read.tags;
      const tileRequest = (binHours: number, index: number, tags: readonly string[], within: Record<string, HourSpan[]> | undefined): ProductRequest =>
        productRequest({
          sourceId: entry.source.id,
          revision: entry.source.revision,
          product: 'series-tiles',
          params: keyed && byId
            ? {
                branchId: entry.branch,
                family: read.family,
                binHours,
                index,
                ...keyedTileParams(tags.map(id => byId.get(id)!), within, gateSpanRef.current),
                ...(read.marks ? { withMarks: true } : {}),
              }
            : {
                branchId: entry.branch,
                family: read.family,
                binHours,
                index,
                countries: tags.map(countryFromKey),
                ...(within
                  ? { within: tags.map(tag => [countryFromKey(tag), (within[tag] ?? []).flatMap(span => gateSpanRef.current(span.firstHour, span.lastHour) ?? [])] as const) }
                  : {}),
                ...(read.marks ? { withMarks: true } : {}),
              },
        });
      const ask = async (binHours: number, index: number, tags: readonly string[], within: Record<string, HourSpan[]> | undefined) => {
        const request = tileRequest(binHours, index, tags, within);
        const reply = validateProductReply(await runtime.client.product<TileReply>(request, new AbortController().signal), request).data;
        return keyed && byId ? keyedTileReply(reply as Parameters<typeof keyedTileReply>[0], tags.map(id => byId.get(id)!)) : reply;
      };
      const promote = prepared ? (...asked: Parameters<typeof tileRequest>) => prepareGate(runtime.client).promote(tileRequest(...asked)) : undefined;
      const laneKey = `${entry.source.id}|${entry.source.revision}|${entry.branch}|${JSON.stringify(read.family)}${keyed ? '|keyed' : ''}`;
      const readTop = read.top;
      const top = readTop
        ? {
            ask: async (binHours: number, index: number, hour: number) => {
              const gated = gateRecordedRef.current(hour);
              if (gated === null) throw new Error('The displayed hour is outside the recording.');
              const request = productRequest({
                sourceId: entry.source.id,
                revision: entry.source.revision,
                product: 'series-tiles',
                params: { branchId: entry.branch, family: read.family, binHours, index, top: readTop.n, ...(readTop.least ? { least: true } : {}), ...(readTop.by ? { rankBy: readTop.by } : {}), hour: gated, ...(read.marks ? { withMarks: true } : {}) },
              });
              return validateProductReply(await runtime.client.product<TileReply>(request, new AbortController().signal), request).data;
            },
          }
        : undefined;
      const tv = new TileView(
        tileCache(runtime.client),
        { key: laneKey, tags: laneTags, within: read.within, lastHour, marks: read.marks === true, ask, prepared, promote, ...(top ? { top } : {}) },
        publish,
      );
      views.set(entry.name, tv);
      if (seriesViewportRef.current) tv.setView(seriesViewportRef.current);
    }
    tileViewsRef.current = views;
    return () => {
      disposed = true;
      for (const tv of views.values()) tv.close();
      if (tileViewsRef.current === views) tileViewsRef.current = new Map();
    };
  }, [runtime.client, active, pixelBinnedKey, lastHour]);

  const onSeriesViewport = useCallback((nextViewport: TileViewport) => {
    seriesViewportRef.current = nextViewport;
    for (const tv of tileViewsRef.current.values()) tv.setView(nextViewport);
  }, []);

  const retryTiles = useCallback(() => {
    for (const tv of tileViewsRef.current.values()) tv.retry();
  }, []);

  const hoverBrokerRef = useRef<FamilySourceBroker | null>(null);
  const [heldHovers, setHeldHovers] = useState<Readonly<Record<string, HeldHover>>>({});
  const asks = useMemo(() => (active && plan ? hoverAsks(plan, hoverHour) : []), [active, plan, hoverHour]);
  const hovers = useMemo(() => currentHovers(asks, heldHovers), [asks, heldHovers]);
  const hoverSlotsRef = useRef<ReadonlySet<string>>(new Set());

  useEffect(() => {
    if (!hoverBrokerRef.current) hoverBrokerRef.current = new FamilySourceBroker();
    const broker = hoverBrokerRef.current;
    const names = new Set(asks.map(ask => ask.name));
    for (const name of hoverSlotsRef.current) if (!names.has(name)) broker.cancelSlot(`hover:${name}`);
    hoverSlotsRef.current = names;
    setHeldHovers(prev => {
      const gone = Object.keys(prev).filter(name => !names.has(name));
      if (!gone.length) return prev;
      const kept = { ...prev };
      for (const name of gone) delete kept[name];
      return kept;
    });
    for (const ask of asks) {
      broker.runSlot(
        `hover:${ask.name}`,
        ask.key,
        async signal => {
          const reply = validateProductReply(await runtime.client.product(ask.request, signal), ask.request);
          return ask.validate ? ask.validate(reply.data) : reply.data;
        },
        data => {
          setHeldHovers(prev => ({ ...prev, [ask.name]: { key: ask.key, hour: ask.hour, data } }));
        },
      );
    }
  }, [asks, runtime.client]);

  useEffect(() => () => {
    hoverBrokerRef.current?.dispose();
    hoverBrokerRef.current = null;
  }, []);

  interface HeldPagedEntry {
    readonly identity: string;
    readonly headLimit: number;
    readonly pageSize: number;
    frozenHour: number | null;
    readonly pages: Map<number, readonly DatedLogRow[]>;
    readonly refusals: Map<number, string>;
    readonly inFlight: Set<number>;
  }
  const pageBrokerRef = useRef<FamilySourceBroker | null>(null);
  const pagedHeldRef = useRef<{ sig: string; entries: Map<string, HeldPagedEntry> }>({ sig: '', entries: new Map() });
  const [pagedRowsState, setPagedRowsState] = useState<{ readonly sig: string; readonly map: Readonly<Record<string, PagedRowsSnapshot>> }>({ sig: '', map: {} });
  const pagedPlanRef = useRef<{ plan: SourcePlan | null; sig: string; active: boolean }>({ plan, sig, active });
  pagedPlanRef.current = { plan, sig, active };

  const publishPagedRows = useCallback((forSig: string) => {
    const held = pagedHeldRef.current;
    if (held.sig !== forSig) return;
    const map: Record<string, PagedRowsSnapshot> = {};
    for (const [name, state] of held.entries) {
      map[name] = {
        identity: state.identity,
        frozenHour: state.frozenHour,
        headLimit: state.headLimit,
        pageSize: state.pageSize,
        pages: Object.fromEntries(state.pages),
        refusals: Object.fromEntries(state.refusals),
      };
    }
    setPagedRowsState({ sig: forSig, map });
  }, []);

  useEffect(() => {
    pageBrokerRef.current?.dispose();
    pageBrokerRef.current = null;
    pagedHeldRef.current = { sig, entries: new Map() };
    setPagedRowsState({ sig, map: {} });
  }, [sig]);

  useEffect(() => () => {
    pageBrokerRef.current?.dispose();
    pageBrokerRef.current = null;
  }, []);

  const onRowWindow = useCallback((rowWindow: FamilyRowWindow, installedHour: number | null) => {
    const { plan: curPlan, sig: curSig, active: curActive } = pagedPlanRef.current;
    if (!curPlan || !curActive) return;
    const pagedEntries = curPlan.entries.filter(
      (entry): entry is SourceEntry & { readonly read: Extract<SourceEntry['read'], { shape: 'paged-rows' }> } => entry.read.shape === 'paged-rows',
    );
    if (!pagedEntries.length) return;
    if (pagedHeldRef.current.sig !== curSig) {
      pageBrokerRef.current?.dispose();
      pageBrokerRef.current = null;
      pagedHeldRef.current = { sig: curSig, entries: new Map() };
    }
    if (!pageBrokerRef.current) pageBrokerRef.current = new FamilySourceBroker();
    const pageBroker = pageBrokerRef.current;
    const first = Math.max(0, Math.floor(rowWindow.first));
    const count = Math.max(0, Math.floor(rowWindow.count));
    const end = first + count;
    let changed = false;

    for (const entry of pagedEntries) {
      const headLimit = Math.max(1, entry.read.tailQuery.limit);
      const pageSize = Math.max(1, entry.read.pageSize ?? DEFAULT_TAIL_PAGE_SIZE);
      const identity = `${entry.source.id}|${entry.branch}|${entry.name}`;
      let state = pagedHeldRef.current.entries.get(entry.name);
      if (!state) {
        state = { identity, headLimit, pageSize, frozenHour: null, pages: new Map(), refusals: new Map(), inFlight: new Set() };
        pagedHeldRef.current.entries.set(entry.name, state);
      }

      if (first === 0 && end <= headLimit) {
        if (state.frozenHour !== null || state.pages.size > 0 || state.refusals.size > 0 || state.inFlight.size > 0) {
          for (const offset of state.inFlight) pageBroker.cancelSlot(`page:${entry.name}:${offset}`);
          state.inFlight.clear();
          state.frozenHour = null;
          state.pages.clear();
          state.refusals.clear();
          changed = true;
        }
        continue;
      }

      if (end <= headLimit || installedHour === null || !Number.isFinite(installedHour)) continue;
      const hour = Math.floor(installedHour);
      if (state.frozenHour !== null && hour < state.frozenHour) {
        for (const offset of state.inFlight) pageBroker.cancelSlot(`page:${entry.name}:${offset}`);
        state.inFlight.clear();
        state.frozenHour = null;
        state.pages.clear();
        state.refusals.clear();
        changed = true;
      }
      if (state.frozenHour === null) {
        state.frozenHour = hour;
        changed = true;
      }
      const frozenHour = state.frozenHour;
      const wanted = new Set(tailPageOffsets({ first, count }, headLimit, pageSize));

      for (const offset of [...state.inFlight]) {
        if (!wanted.has(offset)) {
          state.inFlight.delete(offset);
          pageBroker.cancelSlot(`page:${entry.name}:${offset}`);
        }
      }

      for (const offset of wanted) {
        if (state.pages.has(offset) || state.refusals.has(offset) || state.inFlight.has(offset)) continue;
        const req = buildTailPageRequest(entry, { offset, limit: pageSize, hour: frozenHour });
        if (!req) continue;
        state.inFlight.add(offset);
        const heldState = state;
        const validate = entry.read.validate ?? validateGenericTailReply;
        pageBroker.runSlot(
          `page:${entry.name}:${offset}`,
          `${curSig}|${entry.name}|${frozenHour}|${offset}|${pageSize}`,
          async signal => {
            const reply = validateProductReply(await runtime.client.product(req, signal), req);
            return validate(reply.data, frozenHour);
          },
          tail => {
            if (pagedHeldRef.current.sig !== curSig || heldState.frozenHour !== frozenHour) return;
            heldState.inFlight.delete(offset);
            heldState.refusals.delete(offset);
            heldState.pages.set(offset, tail.rows);
            publishPagedRows(curSig);
          },
          error => {
            if (pagedHeldRef.current.sig !== curSig || heldState.frozenHour !== frozenHour) return;
            heldState.inFlight.delete(offset);
            heldState.refusals.set(offset, asFailure(error).message);
            publishPagedRows(curSig);
          },
        );
      }
    }
    if (changed) publishPagedRows(curSig);
  }, [runtime.client, publishPagedRows]);

  const pagedRows = useMemo(
    () => (pagedRowsState.sig === sig && Object.keys(pagedRowsState.map).length ? pagedRowsState.map : undefined),
    [pagedRowsState, sig],
  );

  const currentTiled = tiledState.key === pixelBinnedKey ? tiledState : null;
  const trackOffset = track?.axisOffset ?? 0;

  const [tileCrossings, setTileCrossings] = useState(0);
  const tileSeenHour = useMemo(
    () => Math.floor(runtime.clock.getHour()) + trackOffset,
    [currentTiled, tileCrossings, runtime.clock, trackOffset],
  );
  const tileHour = following ? tileSeenHour : Math.floor(pinHour ?? window?.lastHour ?? effectiveEndHour);
  useEffect(() => {
    const tops = [...tileViewsRef.current.values()].filter(tv => tv.isTop);
    if (!active || !tops.length) return;
    const now = () => (following ? Math.floor(runtime.clock.getHour()) + trackOffset : tileHour);
    for (const tv of tops) tv.setHour(now());
    if (!following) return;
    const proof = (): readonly [number, number] => {
      let lo = -Infinity;
      let hi = Infinity;
      for (const tv of tops) {
        const held = tv.top;
        lo = Math.max(lo, held ? held.from : now());
        hi = Math.min(hi, held ? held.until : now());
      }
      return [lo - trackOffset, hi + 1 - trackOffset];
    };
    return watchBoundary(runtime.clock, proof, () => { for (const tv of tops) tv.setHour(now()); });
  }, [active, following, tileHour, pixelBinnedKey, currentTiled?.revision, runtime.clock, trackOffset]);
  const tileUnrecorded = useMemo(() => {
    if (!currentTiled) return null;
    const lines: UnrecordedLine[] = [];
    for (const series of Object.values(currentTiled.series)) for (const line of series.values()) lines.push(line);
    return lines.length ? boundedUnrecorded(lines, tileHour) : null;
  }, [currentTiled, tileHour]);
  const tileLo = tileUnrecorded?.validInterval[0];
  const tileHi = tileUnrecorded?.validInterval[1];
  useEffect(() => {
    if (!following || !active || tileLo === undefined || tileHi === undefined) return;
    const now = Math.floor(runtime.clock.getHour()) + trackOffset;
    if (now < tileLo || now > tileHi) {
      setTileCrossings(value => value + 1);
      return;
    }
    if (tileLo === -Infinity && tileHi === Infinity) return;
    return watchBoundary(
      runtime.clock,
      () => [tileLo - trackOffset, tileHi + 1 - trackOffset],
      () => { setTileCrossings(value => value + 1); },
    );
  }, [following, active, runtime.clock, trackOffset, tileLo, tileHi]);

  const entries = useMemo(
    () => unitEntries(plan, following, compiledFollow.subscriptions),
    [plan, following, compiledFollow.subscriptions],
  );
  const frozenGate = useMemo(
    () => freezeGate(gate, track ? (hour: number) => coverageSpan(track, hour) : undefined),
    [gate.open, track, countryTag, singleCountry?.aliases, singleCountry?.historyHeld],
  );
  const content = useContentRevision(runtime.client);
  const displayHour = pinHour ?? Math.floor(window?.lastHour ?? effectiveEndHour);
  const branch = track?.branch;
  const subscribed = following && compiledFollow.subscriptions.length > 0;

  const perfWidget = measuredWidget();
  const readInputs = useRef({ client: runtime.client, source: track?.source, content, perfWidget, clock: runtime.clock, trackOffset });
  readInputs.current = { client: runtime.client, source: track?.source, content, perfWidget, clock: runtime.clock, trackOffset };
  const targetRef = useRef<ReadTarget | null>(null);
  const readTarget = useCallback((): ReadTarget => targetRef.current ??= new ReadTarget(
    {
      now: () => Math.floor(readInputs.current.clock.getHour()) + readInputs.current.trackOffset,
      playing: () => readInputs.current.clock.getSnapshot().playing,
      seek: () => readInputs.current.clock.getSnapshot().seekSequence,
      subscribe: changed => readInputs.current.clock.subscribe(changed),
      watch: ([lo, hi], left) => {
        const { clock, trackOffset: offset } = readInputs.current;
        return watchBoundary(clock, () => [lo - offset, hi + 1 - offset], left);
      },
    },
    (unit, signal, restart) => {
      const { client, source, perfWidget: widgetId } = readInputs.current;
      countRead(widgetId, unit.spec!.request);
      return loadProduct(client, unit.spec!.request, signal, unit.spec!.validate, source, restart, undefined, undefined, true);
    },
    unit => {
      const { client, content: revision } = readInputs.current;
      return peekProduct(client, unit.spec!.request, productKey(unit.spec!.request, revision), unit.spec!.validate);
    },
  ), []);
  useEffect(() => () => {
    targetRef.current?.dispose();
    targetRef.current = null;
  }, []);
  const playback = useMemo(() => {
    if (!branch || !track) return null;
    const own = { branch, effectiveEndHour, gate: frozenGate, countriesKnown: countriesState.known };
    const others = new Map<string, typeof own | Omit<typeof own, 'countriesKnown'>>();
    return (entry: SourceEntry) => {
      if (entry.source.id === track.source.id && entry.branch === track.branch.id) return own;
      const key = JSON.stringify([entry.source.id, entry.branch]);
      let made = others.get(key);
      if (!made) {
        const other = runtime.tracks.find(item => item.source.id === entry.source.id && item.branch.id === entry.branch);
        made = other ? { branch: other.branch, effectiveEndHour: lastRecordedHour(other) ?? other.branch.lastHour, gate: trackGate(other) } : own;
        others.set(key, made);
      }
      return made;
    };
  }, [branch, track, effectiveEndHour, frozenGate, countriesState.known, runtime.tracks]);
  const pinned = useMemo(
    () => (branch ? { timeMode: widget.time.mode, branch, window, pinHour, gate: frozenGate, countriesKnown: countriesState.known } : null),
    [branch, widget.time.mode, window, pinHour, frozenGate, countriesState.known],
  );
  useLayoutEffect(() => {
    const common = { entries: branch ? entries : NO_ENTRIES, active, content, trackOffset };
    if (following && playback) readTarget().configure({ ...common, following: true, subscribed, playback });
    else if (!following && pinned) readTarget().configure({ ...common, following: false, displayHour, pinned });
    else readTarget().configure({ ...common, entries: NO_ENTRIES, following: false, displayHour, pinned: NO_PIN });
  }, [readTarget, branch, entries, active, content, trackOffset, following, subscribed, playback, pinned, displayHour]);
  const subscribeView = useCallback((changed: () => void) => readTarget().subscribe(() => inFrame(changed)), [readTarget]);
  const readView = useCallback(() => targetRef.current?.view() ?? NO_VIEW, []);
  const view = useSyncExternalStore(subscribeView, readView, readView);
  const retryUnits = useCallback(() => { targetRef.current?.retry(); }, []);
  const staticProof = useMemo<StaticProof>(() => ({
    read: () => targetRef.current?.proof() ?? null,
    subscribe: changed => readTarget().subscribeProof(changed),
  }), [readTarget]);

  const shownSpan = view.bundle?.span ?? null;
  const askedSpan = shownSpan ? `${shownSpan[0]}:${shownSpan[1]}` : '';
  const ctxWithStaticSpan = useMemo<FamilyPlanContext<TAux> | null>(() => {
    if (!ctx) return null;
    if (!askedSpan) return ctx;
    const [lo, hi] = askedSpan.split(':').map(Number) as [number, number];
    return { ...ctx, staticSpan: [lo, hi] as const };
  }, [ctx, askedSpan]);

  const currentCountryLooks = wantsColours ? labels : undefined;
  const currentExtra = useMemo<TExtra | undefined>(
    () => (ctxWithStaticSpan && captureExtra ? captureExtra(ctxWithStaticSpan, currentCountryLooks) : undefined),
    [ctxWithStaticSpan, captureExtra, currentCountryLooks],
  );

  interface FrozenView {
    readonly ctx: FamilyPlanContext<TAux> | null;
    readonly gate: FrozenGate;
    readonly plan: SourcePlan | null;
    readonly countryLooks: CountryLooksMap | undefined;
    readonly extra: TExtra | undefined;
  }

  const currentFrozen = useMemo<FrozenView>(
    () => ({ ctx: ctxWithStaticSpan, gate: frozenGate, plan, countryLooks: currentCountryLooks, extra: currentExtra }),
    [ctxWithStaticSpan, frozenGate, plan, currentCountryLooks, currentExtra],
  );

  const viewBundle = view.bundle;
  const viewedBundle = useMemo<StaticBundle | null>(
    () => (viewBundle
      ? { hour: viewBundle.hour, span: viewBundle.span, ...(viewBundle.follows ? { follows: true } : {}), panels: Object.fromEntries(viewBundle.panels), ...(viewBundle.shifts ? { shifts: viewBundle.shifts } : {}) }
      : null),
    [viewBundle],
  );

  const shownRef = useRef<ShownBundle<StaticBundle, FrozenView> | null>(null);
  const shown = showBundle(shownRef.current, viewedBundle, view.held, sig, currentFrozen);
  shownRef.current = shown.last;
  const staticBundle = shown.shown?.bundle ?? null;
  const staticHeld = view.held && staticBundle !== null;
  const displayed = shown.shown?.frozen ?? currentFrozen;
  const displayedCtx = displayed.ctx;
  const displayedGate = displayed.gate;
  const displayedPlan = displayed.plan ?? plan;
  const displayedExtra = displayed.extra;
  const displayedCountryLooks = displayed.countryLooks;

  const exactReadView = useMemo(() => {
    if (!displayedCtx || !displayedPlan || !perfLog) return undefined;
    const frozenPlan = displayedPlan;
    const frozenBranch = displayedCtx.branch;
    const frozenStaticSpan = displayedCtx.staticSpan;
    const frozenClient = runtime.client;
    const frozenTimeMode = widget.time.mode;
    const frozenFollowing = following;
    const frozenWindow = window;
    const g = displayedGate;
    return (wantedHour: number, rowWindow: FamilyRowWindow | null) => readPlanExact(frozenPlan, wantedHour, {
      client: frozenClient,
      following: frozenFollowing,
      timeMode: frozenTimeMode,
      branch: frozenBranch,
      window: frozenWindow,
      staticSpan: frozenStaticSpan,
      gate: g,
      exactOnce: (key, load) => perfLog!.exactOnce(key, load),
      identity: identityProvider,
      art: artProvider,
      rowWindow,
    });
  }, [displayedCtx, displayedPlan, displayedGate, runtime.client, following, widget.time.mode, window, identityProvider, artProvider]);

  const loadingLabel = `Loading ${subject}…`;
  const hasStaticBundle = staticBundle !== null;

  const bodyStatus = useMemo<FamilyBodyStatus>(() => {
    if (!track) return { kind: 'unbound', message: emptyCampaign };
    if (isSingle) {
      if (!gate.open) {
        const s = gate.stage;
        if (s.stage === 'outside') {
          return s.outside.kind === 'not-recorded'
            ? { kind: 'not-recorded', reason: s.outside.message, detail: s.outside.detail }
            : { kind: 'outside', outside: s.outside };
        }
        if (s.stage === 'choose') return { kind: 'choose', message: policy.choose ?? `Choose a country to see its ${subject}.` };
        if (s.stage === 'holders') return { kind: 'holders', tag: singleCountry?.chosen ?? null, holders: s.holders, pick: s.pick };
        if (s.stage === 'absent') {
          const msg = nameFailureDetail ? 'This country does not exist at this date.' : `${countryName} does not exist at this date.`;
          return { kind: 'absent', message: s.reason ?? msg, detail: nameFailureDetail };
        }
        if (s.stage === 'failed') return { kind: 'failed', failure: s.failure, origin: 'local', onRetry: s.retry };
        return { kind: 'loading', label: loadingLabel };
      }
    } else if (coverage.outside) {
      return coverage.outsideKind === 'not-recorded'
        ? { kind: 'not-recorded', reason: coverage.outsideMessage ?? 'Not recorded at this time.', detail: coverage.outsideDetail }
        : {
            kind: 'outside',
            outside: {
              kind: coverage.outsideKind,
              message: coverage.outsideMessage ?? 'This time is outside the recording.',
              detail: coverage.outsideDetail,
              span: coverage.outsideSpan,
              pinned: coverage.outsidePinned,
            },
          };
    }

    if (compiledFollow.refused) return { kind: 'refused', reason: compiledFollow.refused, technical: compiledFollow.refused };
    if (auxEnabled && auxResult.error) return { kind: 'failed', failure: auxResult.error, origin: 'local', onRetry: auxResult.retry };
    if (needsGroups && groups.error && !groups.data) return { kind: 'failed', failure: groups.error, origin: 'local', onRetry: groups.retry };

    if (isFilter) {
      if (filterMatch?.reason) return { kind: 'refused', reason: filterMatch.reason };
      if (countriesState.reason) return { kind: 'refused', reason: countriesState.reason };
      if (!filterMatch?.ready || !groups.data) return { kind: 'loading', label: loadingLabel };
    }

    if (isMulti) {
      if (needsSlots && slotState.failure && !slots) {
        return { kind: 'failed', failure: slotState.failure, origin: 'local', onRetry: slotState.retry };
      }
      if ((needsGroups && !groups.data) || (needsSlots && !slots)) {
        return { kind: 'loading', label: loadingLabel };
      }
      if (countriesState.reason) return { kind: 'empty', message: countriesState.reason };
      if (effectiveMulti && !following) {
        const none = noMembersAt(widget.countries, effectiveMulti, groups.data, moment, lastHour);
        if (none) return { kind: 'no-members', reason: none.reason };
        if (effectiveMulti.top == null && !countriesState.rankedAt(moment).length && !policy.allowSingleSlot) {
          const why = groups.data && widget.countries.kind === 'group' ? worldFilter(widget.countries, [], groups.data, moment, lastHour).reason : null;
          return { kind: 'refused', reason: why ?? 'No countries available for this selection.' };
        }
      }
    }

    if (auxEnabled && auxResult.data === undefined) return { kind: 'loading', label: loadingLabel };

    if (pixelBinnedKey) {
      if (currentTiled?.error) {
        return { kind: 'failed', failure: asFailure(currentTiled.error), origin: 'local', onRetry: retryTiles };
      }
      if (currentTiled?.topRefused) {
        return { kind: 'not-recorded', reason: currentTiled.topRefused };
      }
      if (tileUnrecorded?.reason) {
        return { kind: 'not-recorded', reason: tileUnrecorded.reason };
      }
    }

    const unitState = (): FamilyBodyStatus => {
      if (view.failure) return { kind: 'failed', failure: view.failure.failure, origin: 'local', onRetry: retryUnits };
      if (view.kind === 'outside') return { kind: 'outside' };
      return hasStaticBundle ? { kind: 'ready' } : { kind: 'loading', label: loadingLabel };
    };

    if (following) {
      if (compiledFollow.unavailable) return { kind: 'refused', reason: compiledFollow.unavailable };
      return view.kind === 'none' ? { kind: 'ready' } : unitState();
    }

    if (pixelBinnedKey) {
      if (pinHour === null && !window) return { kind: 'outside' };
      if (!currentTiled || !Object.keys(currentTiled.series).length) return { kind: 'loading', label: loadingLabel };
      return view.kind === 'none' || view.kind === 'outside' ? { kind: 'ready' } : unitState();
    }
    if (view.kind === 'none') return pinHour === null ? { kind: 'outside' } : { kind: 'loading', label: loadingLabel };
    return unitState();
  }, [
    track,
    emptyCampaign,
    isSingle,
    gate.open,
    gate.stage,
    policy,
    subject,
    countryName,
    nameFailureDetail,
    loadingLabel,
    coverage,
    compiledFollow.refused,
    compiledFollow.unavailable,
    auxEnabled,
    auxResult.error,
    auxResult.retry,
    auxResult.data,
    needsGroups,
    groups.error,
    groups.data,
    groups.retry,
    isFilter,
    filterMatch,
    countriesState,
    isMulti,
    needsSlots,
    slotState.failure,
    slotState.retry,
    slots,
    effectiveMulti,
    following,
    widget.countries,
    moment,
    lastHour,
    window,
    pinHour,
    pixelBinnedKey,
    currentTiled,
    tileUnrecorded,
    retryTiles,
    hasStaticBundle,
    view,
    retryUnits,
  ]);

  const bannerFailure: BannerFailureInfo | null = (isSingle && singleCountry?.chosen && singleCountry.error && singleCountry.historyHeld)
    ? { failure: singleCountry.error, retry: singleCountry.retry }
    : ((isFilter || needsGroups) && groups.error && groups.data)
      ? { failure: groups.error, retry: groups.retry }
      : null;

  const bounds = useMemo<readonly [number, number] | null>(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] : null),
    [track?.branch.firstHour, track?.branch.lastHour],
  );

  const pin = following ? null : (staticBundle?.hour ?? pinHour ?? (pixelBinnedKey ? window?.lastHour ?? null : null));

  return {
    track,
    options,
    following,
    owner,
    ctx: ctxWithStaticSpan,
    displayedCtx,
    displayedExtra,
    displayedCountryLooks,
    plan: displayedPlan,
    subscriptions: compiledFollow.subscriptions,
    allowEmptyFollow: compiledFollow.allowEmptyFollow,
    followRefused: compiledFollow.refused,
    staticBundle,
    staticProof,
    staticHeld,
    pagedRows,
    onRowWindow,
    tiledSeries: currentTiled?.series ?? {},
    tiledRevision: currentTiled?.revision ?? 0,
    onSeriesViewport,
    retryTiles,
    hovers,
    exactReadView,
    bodyStatus,
    bannerFailure,
    nameFailureDetail,
    groupWarnings: groups.data?.warnings ?? [],
    moment,
    bounds,
    pin,
    offset: track?.axisOffset ?? 0,
    loadingLabel,
    identitySnapshot,
    identityProvider,
    artProvider,
    onIdentityDemands,
    measureViewport,
  };
}
