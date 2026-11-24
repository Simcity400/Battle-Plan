import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { IconName } from '../../components/Icons';
import { validateProductReply, type ProductRequest } from '../../dashboard/client.ts';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { useHeldTimeline } from '../../dashboard/countryNamesAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { useSlotTagsState } from '../../dashboard/countrySlots.ts';
import { lookStarts } from '../../dashboard/countryTimeline';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import type { FollowedAnswer } from '../../dashboard/follow/message';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import { inFrame } from '../../dashboard/frameUpdates.ts';
import { countryMatch, countryMatchChangeHours, groupUnobserved, membershipUnrecordedAt, noMembersAt, rankedCandidates, resolveCountries, worldFilter } from '../../dashboard/groups';
import { resolveWindow, type WidgetSpec } from '../../dashboard/model';
import { resolveOptions, type OptionField, type OptionValue } from '../../dashboard/options';
import { prepareGate, preparing } from '../../dashboard/prepare.ts';
import { useFollowedProduct, useProduct } from '../../dashboard/products';
import { asFailure, type Failure } from '../../dashboard/queryError.ts';
import { useCountryNameAt } from '../../dashboard/reachedProduct';
import { useGate } from '../../dashboard/regionGate';
import { axisAt, lastRecordedHour, SPEC } from '../../dashboard/regionTime';
import { productRequest } from '../../dashboard/request.ts';
import type { TileReply } from '../../dashboard/tiles/tile.ts';
import { TileView, tileCache, unrecordedReason, type TiledLine, type TileViewport } from '../../dashboard/tiles/view.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useRegion, useRegionCoverage } from '../../dashboard/widgetTime';
import type { HourSpan } from '../../generated/dashboard';
import { perfLog } from '../../shell/perf.ts';
import { createBoundedReadView, displayedHour } from '../boundedView.ts';
import { buildStaticRequest, compileFollowPlan, FamilySourceBroker, planSignature, primaryStaticEntry, readPlanExact } from '../broker.ts';
import type { HolderRun } from '../holders.ts';
import { overlapsHourRanges } from '../holders.ts';
import type { CountryLooksMap } from '../looks.ts';
import type {
  AuxProductSpec,
  BoundedReadView,
  CompiledFollowSubscription,
  CountryPolicy,
  DeclaredCountryFilter,
  DeclaredCountryState,
  FamilyBodyStatus,
  FamilyPlanContext,
  FamilyViewportRequest,
  SourcePlan,
} from '../plan.ts';
import {
  buildDatedCountryLooks,
  effectiveCountryUnobserved,
  extractFactionData,
  freezeGate,
  resolveDisplayedStaticSnapshot,
  selectionUnobservedReason,
  type DisplayedStaticSnapshot,
  type FrozenGate,
} from '../sources.ts';

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

export interface StaticBundle {
  readonly hour: number;
  readonly span: readonly [number, number] | null;
  readonly panels: Readonly<Record<string, unknown>>;
  readonly answers?: Readonly<Record<string, FollowedAnswer>>;
  readonly readView: BoundedReadView;
}

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
  readonly staticHeld: boolean;
  readonly tiledSeries: Readonly<Record<string, ReadonlyMap<string, TiledLine>>>;
  readonly tiledRevision: number;
  readonly onSeriesViewport: (viewport: TileViewport) => void;
  readonly retryTiles: () => void;
  readonly hovers: Readonly<Record<string, { readonly hour: number; readonly data: unknown }>>;
  readonly exactReadView: ((hour: number) => Promise<BoundedReadView | null>) | undefined;
  readonly bodyStatus: FamilyBodyStatus;
  readonly bannerFailure: BannerFailureInfo | null;
  readonly nameFailureDetail: string | undefined;
  readonly groupWarnings: readonly string[];
  readonly moment: number;
  readonly bounds: readonly [number, number] | null;
  readonly pin: number | null;
  readonly offset: number;
  readonly loadingLabel: string;
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
  } = input;

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
    () => (auxSpec && auxEnabled && track ? auxSpec.params(track, options) : null),
    [auxSpec, auxEnabled, track, options],
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
  const slotState = useSlotTagsState(runtime.client, track, active && needsSlots);
  const slots = slotState.slots;

  const effectiveMulti = useMemo(() => {
    if (!resolvedMulti) return null;
    if (typeof singleSlotId !== 'number') return resolvedMulti;
    const runs = slots?.tagRuns(singleSlotId) ?? [];
    return {
      ...resolvedMulti,
      tags: [...new Set(runs.map(run => run.tag))],
      membership: new Map([['', runs]]),
      known: (from: number, to?: number) => !!slots && resolvedMulti.known(from, to),
    };
  }, [resolvedMulti, singleSlotId, slots]);

  const holders = useMemo<Readonly<Record<string, readonly HolderRun[]>> | null>(() => {
    if (!isMulti || !effectiveMulti || effectiveMulti.top != null || !slots) return null;
    const out: Record<string, readonly HolderRun[]> = {};
    for (const key of effectiveMulti.tags) out[key] = slots.runsForKey(key);
    return out;
  }, [isMulti, effectiveMulti, slots]);

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

  const timeline = useHeldTimeline(runtime.client, wantsLabels ? track : undefined, useContentRevision(runtime.client, active && wantsLabels), active && wantsLabels);
  const startsText = wantsLabels ? JSON.stringify(labelTags.map(tag => [tag, lookStarts(timeline, tag, firstHour, lastHour)])) : '[]';
  const starts = useMemo(() => JSON.parse(startsText) as [string, [number, boolean][]][], [startsText]);
  const wantedLabels = useMemo(
    () => (wantsLabels ? starts.flatMap(([tag, hours]) => hours.flatMap(([at, held]) => (held ? [{ tag, hour: at }] : []))) : []),
    [wantsLabels, starts],
  );
  const { exactAt } = useCountryLabelsAt(runtime.client, wantsLabels ? track : undefined, wantedLabels, active && wantsLabels, Infinity, true);
  const builtLooks = wantsLabels ? buildDatedCountryLooks(timeline, labelTags, firstHour, lastHour, exactAt, wantsColours) : NO_LOOKS;
  const labelsText = wantsLabels ? JSON.stringify(builtLooks) : '{}';
  const labels = useMemo<CountryLooksMap>(
    () => (wantsLabels ? JSON.parse(labelsText) as CountryLooksMap : NO_LOOKS),
    [wantsLabels, labelsText],
  );

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
    return buildSources(ctx, { following, staticSpan: null });
  }, [ctx, inputsReady, buildSources, following]);

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
      return [e.name, e.source.id, e.source.revision, e.branch, r.family, r.tags, r.within ? [...r.within.entries()] : null, r.marks === true, lastHour];
    }));
  }, [plan, lastHour]);

  const tileViewsRef = useRef<Map<string, TileView>>(new Map());
  const seriesViewportRef = useRef<TileViewport | null>(null);
  const [tiledState, setTiledState] = useState<{
    readonly key: string;
    readonly series: Readonly<Record<string, ReadonlyMap<string, TiledLine>>>;
    readonly revision: number;
    readonly error: unknown;
    readonly unrecorded: string | null;
  }>({ key: '', series: {}, revision: 0, error: null, unrecorded: null });

  const planForTilesRef = useRef(plan);
  planForTilesRef.current = plan;
  const gateSpanRef = useRef(gate.span);
  gateSpanRef.current = gate.span;

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
      let firstUnrecorded: string | null = null;
      for (const [name, tv] of views) {
        if (tv.current) {
          nextSeries[name] = tv.current;
          if (!firstUnrecorded) {
            const reason = unrecordedReason(tv.current);
            if (reason) firstUnrecorded = reason;
          }
        }
        if (firstError === null && tv.error !== null) firstError = tv.error;
      }
      rev++;
      const snapshot = { key: pixelBinnedKey, series: nextSeries, revision: rev, error: firstError, unrecorded: firstUnrecorded };
      inFrame(() => {
        if (!disposed) setTiledState(snapshot);
      });
    };

    const prepared = preparing(runtime.client);
    for (const entry of entries) {
      const read = entry.read;
      if (!read.tags.length) continue;
      const tileRequest = (binHours: number, index: number, tags: readonly string[], within: Record<string, HourSpan[]> | undefined): ProductRequest =>
        productRequest({
          sourceId: entry.source.id,
          revision: entry.source.revision,
          product: 'series-tiles',
          params: {
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
        return validateProductReply(await runtime.client.product<TileReply>(request, new AbortController().signal), request).data;
      };
      const promote = prepared ? (...asked: Parameters<typeof tileRequest>) => prepareGate(runtime.client).promote(tileRequest(...asked)) : undefined;
      const laneKey = `${entry.source.id}|${entry.source.revision}|${entry.branch}|${JSON.stringify(read.family)}`;
      const tv = new TileView(
        tileCache(runtime.client),
        { key: laneKey, tags: read.tags, within: read.within, lastHour, marks: read.marks === true, ask, prepared, promote },
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

  const onSeriesViewport = useCallback((viewport: TileViewport) => {
    seriesViewportRef.current = viewport;
    for (const tv of tileViewsRef.current.values()) tv.setView(viewport);
  }, []);

  const retryTiles = useCallback(() => {
    for (const tv of tileViewsRef.current.values()) tv.retry();
  }, []);

  const hoverBrokerRef = useRef<FamilySourceBroker | null>(null);
  const [hovers, setHovers] = useState<Readonly<Record<string, { readonly hour: number; readonly data: unknown }>>>({});

  useEffect(() => {
    if (!hoverBrokerRef.current) hoverBrokerRef.current = new FamilySourceBroker();
    const broker = hoverBrokerRef.current;
    const entries = active && plan
      ? plan.entries.filter((e): e is typeof e & { readonly read: Extract<typeof e.read, { shape: 'exact-hover' }> } => e.read.shape === 'exact-hover')
      : [];
    if (!entries.length || hoverHour === null || hoverHour === undefined || !Number.isFinite(hoverHour)) {
      for (const e of entries) broker.cancelSlot(`hover:${e.name}`);
      setHovers(prev => (Object.keys(prev).length ? {} : prev));
      return;
    }
    const targetHour = Math.floor(hoverHour);
    for (const entry of entries) {
      const read = entry.read;
      if (!read.params) continue;
      const params = { ...read.params, [read.hourParam ?? 'hour']: targetHour };
      const req = productRequest({ sourceId: entry.source.id, revision: entry.source.revision, product: read.product, params });
      const key = `${entry.source.id}|${entry.source.revision}|${entry.branch}|${read.product}|${JSON.stringify(params)}`;
      broker.runSlot(
        `hover:${entry.name}`,
        key,
        async signal => {
          const reply = validateProductReply(await runtime.client.product(req, signal), req);
          return read.validate ? read.validate(reply.data) : reply.data;
        },
        data => {
          setHovers(prev => ({ ...prev, [entry.name]: { hour: targetHour, data } }));
        },
      );
    }
  }, [active, plan, hoverHour, runtime.client]);

  useEffect(() => () => {
    hoverBrokerRef.current?.dispose();
    hoverBrokerRef.current = null;
  }, []);

  const currentTiled = tiledState.key === pixelBinnedKey ? tiledState : null;

  const frozenGate = useMemo(
    () => freezeGate(gate),
    [gate.open, track, countryTag, singleCountry?.aliases, singleCountry?.historyHeld],
  );

  const staticSpec = useMemo(() => {
    if (following || !plan || !ctx) return null;
    return buildStaticRequest(plan, {
      timeMode: widget.time.mode,
      branch: ctx.branch,
      window,
      pinHour,
      gate: frozenGate,
      countriesKnown: countriesState.known,
    });
  }, [following, plan, ctx, widget.time.mode, window, pinHour, frozenGate, countriesState.known]);

  const staticValidate = useCallback((raw: unknown) => {
    return staticSpec?.validate ? staticSpec.validate(raw) : raw;
  }, [staticSpec]);

  const dynamicKeys = staticSpec?.dynamicKeys ?? [];
  const staticProduct = useFollowedProduct<unknown>(
    runtime.client,
    staticSpec?.request ?? null,
    dynamicKeys,
    active && !following,
    staticValidate,
    track?.source,
  );

  const staticReqKey = useMemo(() => (staticSpec ? JSON.stringify(staticSpec.request) : null), [staticSpec]);
  const expectedStaticKey = useMemo(() => {
    if (!staticSpec || !dynamicKeys.length) return null;
    const params = staticSpec.request.params as Record<string, unknown>;
    return JSON.stringify(dynamicKeys.map(name => params[name]));
  }, [staticSpec, dynamicKeys]);
  const staticHeld = !following && staticSpec !== null && staticProduct.data !== undefined
    && (staticProduct.asking !== undefined || (expectedStaticKey !== null && staticProduct.moment !== expectedStaticKey));

  const ctxWithStaticSpan = useMemo<FamilyPlanContext<TAux> | null>(
    () => (ctx ? { ...ctx, staticSpan: staticSpec?.span ?? null } : null),
    [ctx, staticSpec?.span],
  );

  const currentCountryLooks = wantsColours ? labels : undefined;
  const currentExtra = useMemo<TExtra | undefined>(
    () => (ctxWithStaticSpan && captureExtra ? captureExtra(ctxWithStaticSpan, currentCountryLooks) : undefined),
    [ctxWithStaticSpan, captureExtra, currentCountryLooks],
  );

  interface FrozenPlanExtra {
    readonly plan: SourcePlan | null;
    readonly countryLooks: CountryLooksMap | undefined;
    readonly extra: TExtra | undefined;
  }

  const currentFrozenExtra = useMemo<FrozenPlanExtra>(
    () => ({ plan, countryLooks: currentCountryLooks, extra: currentExtra }),
    [plan, currentCountryLooks, currentExtra],
  );

  const pendingStaticByMomentRef = useRef(
    new Map<string, { readonly requestKey: string | null; readonly context: FamilyPlanContext<TAux>; readonly gate: FrozenGate; readonly span: readonly [number, number] | null; readonly extra: FrozenPlanExtra }>(),
  );
  if (!following && staticSpec && ctxWithStaticSpan) {
    const key = expectedStaticKey ?? '__single__';
    pendingStaticByMomentRef.current.set(key, {
      requestKey: staticReqKey,
      context: ctxWithStaticSpan,
      gate: frozenGate,
      span: staticSpec.span,
      extra: currentFrozenExtra,
    });
    if (pendingStaticByMomentRef.current.size > 32) {
      pendingStaticByMomentRef.current.delete(pendingStaticByMomentRef.current.keys().next().value!);
    }
  }

  const displayedStaticRef = useRef<DisplayedStaticSnapshot<FamilyPlanContext<TAux>, FrozenPlanExtra> | null>(null);
  const displayedStatic = useMemo(() => {
    if (following || staticProduct.data === undefined || !ctxWithStaticSpan) {
      displayedStaticRef.current = null;
      return null;
    }
    const satisfiesCurrent = staticSpec !== null && !staticHeld;
    const matched = !satisfiesCurrent ? pendingStaticByMomentRef.current.get(staticProduct.moment ?? '__single__') : undefined;
    const sourceCtx = satisfiesCurrent ? ctxWithStaticSpan : (matched?.context ?? ctxWithStaticSpan);
    const sourceGate = satisfiesCurrent ? frozenGate : (matched?.gate ?? frozenGate);
    const sourceExtra = satisfiesCurrent ? currentFrozenExtra : (matched?.extra ?? currentFrozenExtra);
    const requested = satisfiesCurrent ? (staticSpec?.span ?? null) : (matched?.span ?? staticSpec?.span ?? null);
    const next = resolveDisplayedStaticSnapshot(displayedStaticRef.current, {
      data: staticProduct.data,
      moment: staticProduct.moment,
      requestKey: satisfiesCurrent ? staticReqKey : (matched?.requestKey ?? staticReqKey),
      satisfiesCurrent,
      requestedSpan: requested,
      context: sourceCtx,
      gate: sourceGate,
      extra: sourceExtra,
    });
    displayedStaticRef.current = next;
    return next;
  }, [following, staticSpec, staticReqKey, staticHeld, staticProduct.data, staticProduct.moment, ctxWithStaticSpan, frozenGate, currentFrozenExtra]);

  const staticBundle = useMemo<StaticBundle | null>(() => {
    if (following || !displayedStatic) return null;
    const frozenPlan = displayedStatic.extra.plan ?? plan;
    const entry = frozenPlan ? primaryStaticEntry(frozenPlan) : null;
    const read = entry?.read;
    const entryName = entry?.name ?? 'main';
    let shownHour = pinHour ?? effectiveEndHour;
    let shownSpan: readonly [number, number] | null = null;
    if (read?.shape === 'point') {
      if (read.channel === 'panel' && !read.exactProduct && displayedStatic.moment) {
        try {
          const parsed = JSON.parse(displayedStatic.moment) as number[];
          if (typeof parsed[0] === 'number') shownHour = parsed[0];
        } catch {
        }
      } else if (typeof (displayedStatic.data as { hour?: unknown }).hour === 'number') {
        shownHour = (displayedStatic.data as { hour: number }).hour;
      }
    } else if (read?.shape === 'branch-to-hour' || read?.shape === 'trailing-window' || read?.shape === 'range') {
      shownSpan = displayedStatic.span;
      if (shownSpan) shownHour = shownSpan[1];
    }
    const isLivePanel = read?.shape === 'point' && read.channel === 'panel' && !read.exactProduct;
    const live = isLivePanel
      ? new Map([[entryName, repliedCells(displayedStatic.data).read()]])
      : new Map();
    const panelsMap = new Map([[entryName, displayedStatic.data]]);
    return {
      hour: shownHour,
      span: shownSpan,
      panels: { [entryName]: displayedStatic.data },
      readView: createBoundedReadView({
        hour: displayedHour(shownHour),
        panels: panelsMap,
        live,
      }),
    };
  }, [following, displayedStatic, plan, pinHour, effectiveEndHour]);

  const retainedStaticSpan = staticBundle?.span ?? null;
  const displayedCtx = useMemo<FamilyPlanContext<TAux> | null>(() => {
    if (following) return ctxWithStaticSpan;
    if (displayedStatic) return { ...displayedStatic.context, staticSpan: retainedStaticSpan };
    return ctxWithStaticSpan;
  }, [ctxWithStaticSpan, following, displayedStatic, retainedStaticSpan]);
  const displayedGate = !following && displayedStatic ? displayedStatic.gate : frozenGate;
  const displayedPlan = !following && displayedStatic ? (displayedStatic.extra.plan ?? plan) : plan;
  const displayedExtra = !following && displayedStatic ? displayedStatic.extra.extra : currentExtra;
  const displayedCountryLooks = !following && displayedStatic ? displayedStatic.extra.countryLooks : currentCountryLooks;

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
    return (wantedHour: number) => readPlanExact(frozenPlan, wantedHour, {
      client: frozenClient,
      following: frozenFollowing,
      timeMode: frozenTimeMode,
      branch: frozenBranch,
      window: frozenWindow,
      staticSpan: frozenStaticSpan,
      gate: g,
      exactOnce: (key, load) => perfLog!.exactOnce(key, load),
    });
  }, [displayedCtx, displayedPlan, displayedGate, runtime.client, following, widget.time.mode, window]);

  const loadingLabel = `Loading ${subject}…`;

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
      if (needsSlots && slotState.error && !slots) {
        return { kind: 'failed', failure: slotState.error, origin: 'local', onRetry: slotState.retry };
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
      if (currentTiled?.unrecorded) {
        return { kind: 'not-recorded', reason: currentTiled.unrecorded };
      }
    }

    if (following) {
      if (compiledFollow.unavailable) return { kind: 'refused', reason: compiledFollow.unavailable };
      return { kind: 'ready' };
    }

    const staticEntry = plan ? primaryStaticEntry(plan) : null;
    if (staticEntry?.read.shape === 'pixel-binned') {
      if (pinHour === null && !window) return { kind: 'outside' };
      if (!currentTiled || !Object.keys(currentTiled.series).length) return { kind: 'loading', label: loadingLabel };
      return { kind: 'ready' };
    }
    if (staticEntry && (staticEntry.read.shape === 'branch-to-hour' || staticEntry.read.shape === 'trailing-window' || staticEntry.read.shape === 'range')) {
      if (!window || !staticSpec) return { kind: 'outside' };
    } else if (pinHour === null) {
      return { kind: 'outside' };
    }
    if (staticProduct.error) {
      return { kind: 'failed', failure: staticProduct.error, origin: 'local', onRetry: staticProduct.retry };
    }
    if (staticProduct.data === undefined) return { kind: 'loading', label: loadingLabel };
    return { kind: 'ready' };
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
    slotState.error,
    slotState.retry,
    slots,
    effectiveMulti,
    following,
    widget.countries,
    moment,
    lastHour,
    plan,
    window,
    staticSpec,
    pinHour,
    staticProduct.error,
    staticProduct.retry,
    staticProduct.data,
    pixelBinnedKey,
    currentTiled,
    retryTiles,
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

  const staticEntry = displayedPlan ? primaryStaticEntry(displayedPlan) : null;
  const isPanelStatic = staticEntry?.read.shape === 'point' && staticEntry.read.channel === 'panel' && !staticEntry.read.exactProduct;
  const pin = following ? null : (isPanelStatic ? (staticBundle?.hour ?? pinHour) : (staticBundle?.hour ?? (staticEntry?.read.shape === 'pixel-binned' ? (pinHour ?? window?.lastHour ?? null) : null)));

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
    staticHeld,
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
  };
}
