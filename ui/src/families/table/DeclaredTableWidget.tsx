import { useCallback, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import { narrowTableOf, tableSortOf, tableSortToken } from '../../charts/table/options';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { useHeldTimeline } from '../../dashboard/countryNamesAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { useSlotTagsState } from '../../dashboard/countrySlots.ts';
import { lookStarts } from '../../dashboard/countryTimeline';
import type { DashboardClient } from '../../dashboard/client';
import type { WidgetProps } from '../../dashboard/definitions';
import type { Channel } from '../../dashboard/follow/facts';
import { repliedCells } from '../../dashboard/follow/liveCells.ts';
import { usePlaybackHour } from '../../dashboard/follow/useFollow.ts';
import GateState from '../../dashboard/GateState';
import { countryMatch, countryMatchChangeHours, groupUnobserved, membershipUnrecordedAt, noMembersAt, rankedCandidates, resolveCountries, worldFilter } from '../../dashboard/groups';
import { resolveWindow, TRAILING_HOURS, type WidgetTime } from '../../dashboard/model';
import NoMembersState from '../../dashboard/NoMembersState';
import { resolveOptions } from '../../dashboard/options';
import { useFollowedProduct, useProduct } from '../../dashboard/products';
import { useCountryNameAt } from '../../dashboard/reachedProduct';
import { useGate } from '../../dashboard/regionGate';
import { axisAt, SPEC } from '../../dashboard/regionTime';
import { forwardRequest, productRequest } from '../../dashboard/request.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useWidgetSkeleton } from '../../dashboard/widgetFrame.tsx';
import { useRegion, useRegionCoverage } from '../../dashboard/widgetTime';
import { perfLog } from '../../shell/perf.ts';
import type { FamilySources, FamilySourceSubscription } from '../contract.ts';
import type { CountryLooksMap } from '../looks.ts';
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
import {
  resolveHeadSpan,
  type DeclaredCountryFilter,
  type DeclaredCountryState,
  type DeclaredTableContext,
  type HolderRun,
  type TableWidgetDeclaration,
} from './declaration.ts';
import FamilyTableHost from './FamilyTableHost.tsx';
import { encodeTableSettings, TABLE_SETTINGS_KEY, tableSettingsOf, type TableSettings } from './settings.ts';

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

interface FrozenExactSnapshot<TProjectionOptions, TAux> {
  readonly declaration: TableWidgetDeclaration<TProjectionOptions, TAux>;
  readonly client: DashboardClient;
  readonly following: boolean;
  readonly timeMode: WidgetTime['mode'];
  readonly window: { readonly firstHour: number; readonly lastHour: number } | null;
  readonly ctx: DeclaredTableContext<TAux>;
  readonly gate: FrozenGate;
}

function createFrozenExactSources<TProjectionOptions, TAux>(
  frozen: FrozenExactSnapshot<TProjectionOptions, TAux>,
): (wantedHour: number) => Promise<FamilySources | null> {
  return async (wantedHour: number): Promise<FamilySources | null> => {
    if (!perfLog) return null;
    const recorded = frozen.gate.recorded(wantedHour);
    if (recorded === null) return null;
    const src = frozen.declaration.source;
    const ctx = frozen.ctx;
    if (src.kind === 'panel') {
      const params = src.params(ctx);
      if (!params) return null;
      const req = forwardRequest(ctx.track.source, src.product, { ...params, hour: recorded });
      const raw = await perfLog.exactOnce(JSON.stringify(req), async () => (await frozen.client.product(req, new AbortController().signal)).data);
      return { answers: new Map(), panels: new Map([['main', raw]]), live: new Map([['main', repliedCells(raw).read()]]) };
    }
    if (src.kind === 'head') {
      const span = !frozen.following && ctx.staticSpan
        ? frozen.gate.span(ctx.staticSpan[0], ctx.staticSpan[1])
        : (() => {
          const resolved = resolveHeadSpan(
            src.timeShape,
            src.windowHours ?? TRAILING_HOURS,
            frozen.timeMode,
            ctx.branch,
            frozen.window,
            recorded,
          );
          return resolved ? frozen.gate.span(resolved.first, resolved.last) : null;
        })();
      if (!span) return null;
      const params = src.productParams(ctx, span);
      if (!params) return null;
      const req = productRequest({ sourceId: ctx.track.source.id, revision: ctx.track.source.revision, product: src.product, params });
      const raw = await perfLog.exactOnce(JSON.stringify(req), async () => (await frozen.client.product(req, new AbortController().signal)).data);
      return { answers: new Map(), panels: new Map([['main', raw]]), live: new Map() };
    }
    const params = src.exactParams(ctx, recorded, 'exact');
    if (!params) return null;
    const req = productRequest({ sourceId: ctx.track.source.id, revision: ctx.track.source.revision, product: src.exactProduct, params });
    const raw = await perfLog.exactOnce(JSON.stringify(req), async () => (await frozen.client.product(req, new AbortController().signal)).data);
    return { answers: new Map(), panels: new Map([['main', raw]]), live: new Map() };
  };
}

export interface DeclaredTableWidgetProps<TProjectionOptions, TAux> extends WidgetProps {
  readonly declaration: TableWidgetDeclaration<TProjectionOptions, TAux>;
  readonly selectedKey?: string | null;
  readonly onActivate?: (key: string, meta?: unknown, hour?: number) => void;
  readonly extraWarnings?: readonly string[];
}

export default function DeclaredTableWidget<TProjectionOptions, TAux>({
  declaration,
  widget,
  runtime,
  active,
  onDiagnostics,
  setOption,
  headerSlot,
  selectedKey = null,
  onActivate,
  extraWarnings,
}: DeclaredTableWidgetProps<TProjectionOptions, TAux>) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  useWidgetSkeleton('table');
  const following = widget.time.mode === 'follow';
  const options = useMemo(
    () => resolveOptions(declaration.optionFields ?? [], widget.options),
    [declaration.optionFields, widget.options],
  );
  const policy = declaration.countries;
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
  const nameDetail = isSingle && countryLabel.nameFailure
    ? <span data-detail={countryLabel.nameFailure.message}>{countryName}</span>
    : undefined;

  const firstHour = track?.branch.firstHour ?? 0;
  const lastHour = track?.branch.lastHour ?? 0;
  const playbackHour = usePlaybackHour(runtime.clock, false);
  const rawWindow = !following && track ? resolveWindow(widget, track, runtime.tracks, playbackHour) : null;
  const windowFirst = rawWindow?.firstHour ?? null;
  const windowLast = rawWindow?.lastHour ?? null;
  const window = useMemo(
    () => (windowFirst !== null && windowLast !== null ? { firstHour: windowFirst, lastHour: windowLast } : null),
    [windowFirst, windowLast],
  );
  const moment = Math.floor(window?.lastHour ?? lastHour);
  const pinHour = !following && track ? gate.hourAt(axisAt(playbackHour)) : null;

  const auxSpec = declaration.aux;
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
    unobservedIn: (first: number, last: number) => unobserved.some(([spanFirst, spanLast]) => spanFirst <= last && first <= spanLast),
  }), [widget.countries, effectiveMulti, filterUnobserved, holders, allTags, membership, membershipRuns, unobserved, filter, labels, factionData, filterMatch?.reason]);

  const staticRange = useMemo(() => {
    if (following || !track || !window) return null;
    const src = declaration.source;
    if (src.kind !== 'head') return null;
    const resolved = resolveHeadSpan(
      src.timeShape,
      src.windowHours ?? TRAILING_HOURS,
      widget.time.mode,
      track.branch,
      window,
      Math.floor(window.lastHour),
    );
    if (!resolved) return null;
    const { first: rangeFirst, last: rangeLast } = resolved;
    if (src.timeShape === 'branch-to-hour') {
      if (widget.time.mode !== 'range' && gate.recorded(rangeLast) === null) return null;
      const span = gate.span(rangeFirst, rangeLast);
      return span ? { span, first: rangeFirst, last: rangeLast } : null;
    }
    if (!countriesState.known(rangeFirst, rangeLast)) return null;
    const span = gate.span(rangeFirst, rangeLast);
    return span ? { span, first: rangeFirst, last: rangeLast } : null;
  }, [following, track, window, declaration.source, widget.time.mode, gate.recorded, gate.span, countriesState]);

  const baseCtx = useMemo<Omit<DeclaredTableContext<TAux>, 'settings'> | null>(() => {
    if (!track) return null;
    return {
      track,
      branch: track.branch,
      widget,
      options,
      following,
      aux: auxResult.data,
      countryTag,
      countryName,
      countries: countriesState,
      gateSpan: gate.span,
      staticSpan: staticRange ? [staticRange.first, staticRange.last] as const : null,
    };
  }, [track, widget, options, following, auxResult.data, countryTag, countryName, countriesState, gate.span, staticRange]);

  const columns = useMemo(
    () => (typeof declaration.columns === 'function' ? (baseCtx ? declaration.columns(baseCtx) : []) : declaration.columns),
    [declaration.columns, baseCtx],
  );

  const settings = useMemo<TableSettings>(() => {
    const sp = declaration.settings;
    if (sp.kind === 'fixed') return tableSettingsOf(undefined, columns, sp.base);
    if (sp.kind === 'table') return tableSettingsOf(options[sp.key ?? TABLE_SETTINGS_KEY], columns, sp.base);
    if (sp.kind === 'options') {
      const sort = tableSortOf(options.tableSort, sp.defaultSort);
      const overflow = narrowTableOf(options) === 'scroll' ? 'scroll' as const : 'tooltip' as const;
      const density = options.density === 'compact' ? 'compact' as const : 'comfortable' as const;
      return tableSettingsOf(undefined, columns, { density, overflow, sort });
    }
    return baseCtx ? sp.resolve(baseCtx, columns) : tableSettingsOf(undefined, columns, { density: 'comfortable', overflow: 'tooltip', sort: { column: columns[0]?.key ?? '', direction: 'descending' } });
  }, [declaration.settings, columns, options, baseCtx]);

  const ctx = useMemo<DeclaredTableContext<TAux> | null>(
    () => (baseCtx ? { ...baseCtx, settings } : null),
    [baseCtx, settings],
  );

  const onSettings = useCallback((next: TableSettings) => {
    if (!setOption || !ctx) return;
    const sp = declaration.settings;
    if (sp.kind === 'fixed') return;
    if (sp.kind === 'table') {
      setOption(sp.key ?? TABLE_SETTINGS_KEY, encodeTableSettings(next));
      return;
    }
    if (sp.kind === 'options') {
      if (next.sort.column !== settings.sort.column || next.sort.direction !== settings.sort.direction) {
        setOption('tableSort', tableSortToken(next.sort));
      }
      return;
    }
    sp.onChange(next, settings, ctx, setOption);
  }, [setOption, ctx, declaration.settings, settings]);

  const canFollow = following && active && !!ctx
    && (!isSingle || (gate.open && countryTag !== null))
    && (!isFilter || (!!filterMatch?.ready && !filterMatch.reason && !!groups.data))
    && (!auxEnabled || auxResult.data !== undefined)
    && (!isMulti || (!countriesState.reason && (!needsGroups || !!groups.data) && (!needsSlots || !!slots)));

  const followPlan = useMemo(() => {
    if (!canFollow || !ctx) {
      return { subs: [] as FamilySourceSubscription[], refused: null as string | null, unavailable: null as string | null, allowEmptyFollow: false };
    }
    const src = declaration.source;
    const subs: FamilySourceSubscription[] = [];
    let refused: string | null = null;
    const add = (name: string, rawQuery: Record<string, unknown>, channel: Channel, product?: string) => {
      const checked = gate.checkFollow(rawQuery);
      if (checked.refused) refused ??= checked.refused;
      else if (checked.query) {
        subs.push({ name, source: ctx.track.source, branch: ctx.branch.id, offset: ctx.track.axisOffset, query: checked.query, channel, product });
      }
    };
    if (src.kind === 'panel') {
      const params = src.params(ctx);
      if (params) add('main', { panel: src.product, ...params }, 'panel', src.product);
      return { subs, refused, unavailable: null as string | null, allowEmptyFollow: false };
    }
    if (src.kind === 'head') {
      const query = src.followQuery(ctx);
      if (query) add('main', query, 'followed');
      return { subs, refused, unavailable: null as string | null, allowEmptyFollow: false };
    }
    const built = src.follow(ctx);
    for (const item of built.subs) add(item.name, item.query, item.channel ?? 'followed', item.product);
    return { subs, refused, unavailable: built.unavailable ?? null, allowEmptyFollow: built.allowEmptyFollow === true };
  }, [canFollow, ctx, declaration.source, gate.checkFollow]);

  const staticReq = useMemo(() => {
    if (following || !ctx) return null;
    if (isSingle && (!gate.open || !countryTag)) return null;
    if (isFilter && (!filterMatch?.ready || filterMatch.reason || !groups.data)) return null;
    if (auxEnabled && auxResult.data === undefined) return null;
    if (isMulti && (countriesState.reason || (needsGroups && !groups.data) || (needsSlots && !slots))) return null;
    const src = declaration.source;
    if (src.kind === 'panel') {
      if (pinHour === null) return null;
      const params = src.params(ctx);
      return params ? forwardRequest(ctx.track.source, src.product, { ...params, hour: pinHour }) : null;
    }
    if (src.kind === 'head') {
      if (!staticRange) return null;
      const params = src.productParams(ctx, staticRange.span);
      return params ? productRequest({ sourceId: ctx.track.source.id, revision: ctx.track.source.revision, product: src.product, params }) : null;
    }
    if (pinHour === null) return null;
    const params = src.exactParams(ctx, pinHour, 'static');
    return params ? productRequest({ sourceId: ctx.track.source.id, revision: ctx.track.source.revision, product: src.exactProduct, params }) : null;
  }, [following, ctx, isSingle, gate.open, countryTag, isFilter, filterMatch, groups.data, auxEnabled, auxResult.data, isMulti, countriesState.reason, needsGroups, needsSlots, slots, declaration.source, pinHour, staticRange]);

  const staticValidate = useCallback((raw: unknown) => {
    const src = declaration.source;
    if (src.kind === 'multi') return ctx ? src.validate(raw, ctx) : raw;
    return src.validate(raw);
  }, [declaration.source, ctx]);

  const dynamicKeys = declaration.source.kind === 'panel' ? ['hour'] : (declaration.source.dynamicKeys ?? []);
  const staticProduct = useFollowedProduct<unknown>(
    runtime.client,
    staticReq,
    dynamicKeys,
    active && !following,
    staticValidate,
    track?.source,
  );

  const expectedStaticKey = useMemo(() => {
    if (!staticReq || !dynamicKeys.length) return null;
    const params = staticReq.params as Record<string, unknown>;
    return JSON.stringify(dynamicKeys.map(name => params[name]));
  }, [staticReq, dynamicKeys]);
  const staticHeld = !following && staticReq !== null && staticProduct.data !== undefined
    && (staticProduct.asking !== undefined || (expectedStaticKey !== null && staticProduct.moment !== expectedStaticKey));

  const frozenGate = useMemo(
    () => freezeGate(gate),
    [gate.open, track, countryTag, singleCountry?.aliases, singleCountry?.historyHeld],
  );

  const pendingStaticByMomentRef = useRef(new Map<string, { readonly context: DeclaredTableContext<TAux>; readonly gate: FrozenGate; readonly span: readonly [number, number] | null }>());
  if (!following && staticReq && ctx) {
    const key = expectedStaticKey ?? '__single__';
    pendingStaticByMomentRef.current.set(key, {
      context: ctx,
      gate: frozenGate,
      span: staticRange ? [staticRange.first, staticRange.last] as const : null,
    });
    if (pendingStaticByMomentRef.current.size > 32) {
      pendingStaticByMomentRef.current.delete(pendingStaticByMomentRef.current.keys().next().value!);
    }
  }

  const displayedStaticRef = useRef<DisplayedStaticSnapshot<DeclaredTableContext<TAux>> | null>(null);
  const displayedStatic = useMemo(() => {
    if (following || staticProduct.data === undefined || !ctx) {
      displayedStaticRef.current = null;
      return null;
    }
    const matched = pendingStaticByMomentRef.current.get(staticProduct.moment ?? '__single__');
    const sourceCtx = matched?.context ?? ctx;
    const sourceGate = matched?.gate ?? frozenGate;
    const requested = matched?.span ?? (staticRange ? [staticRange.first, staticRange.last] as const : null);
    const next = resolveDisplayedStaticSnapshot(
      displayedStaticRef.current,
      staticProduct.data,
      staticProduct.moment,
      requested,
      sourceCtx,
      sourceGate,
    );
    displayedStaticRef.current = next;
    return next;
  }, [following, staticProduct.data, staticProduct.moment, staticRange, ctx, frozenGate]);

  const staticSources = useMemo<{ readonly sources: FamilySources; readonly hour: number; readonly span?: readonly [number, number] | null } | null>(() => {
    if (following || !displayedStatic) return null;
    const src = declaration.source;
    let shownHour = pinHour ?? lastHour;
    let shownSpan: readonly [number, number] | null = null;
    if (src.kind === 'panel' && displayedStatic.moment) {
      const parsed = JSON.parse(displayedStatic.moment) as number[];
      if (typeof parsed[0] === 'number') shownHour = parsed[0];
    } else if (src.kind === 'head') {
      shownSpan = displayedStatic.span;
      if (shownSpan) shownHour = shownSpan[1];
    } else if (src.kind === 'multi' && typeof (displayedStatic.data as { hour?: unknown }).hour === 'number') {
      shownHour = (displayedStatic.data as { hour: number }).hour;
    }
    const live = src.kind === 'panel'
      ? new Map([['main', repliedCells(displayedStatic.data).read()]])
      : new Map();
    return {
      sources: { answers: new Map(), panels: new Map([['main', displayedStatic.data]]), live },
      hour: shownHour,
      ...(shownSpan ? { span: shownSpan } : {}),
    };
  }, [following, displayedStatic, declaration.source, pinHour, lastHour]);

  const retainedStaticSpan = staticSources?.span ?? null;
  const displayedCtx = useMemo<DeclaredTableContext<TAux> | null>(() => {
    if (following) return ctx;
    if (displayedStatic) return { ...displayedStatic.context, staticSpan: retainedStaticSpan };
    return ctx;
  }, [ctx, following, displayedStatic, retainedStaticSpan]);
  const displayedGate = !following && displayedStatic ? displayedStatic.gate : frozenGate;

  const exactSources = useMemo(() => {
    if (!displayedCtx || !perfLog) return undefined;
    return createFrozenExactSources({
      declaration,
      client: runtime.client,
      following,
      timeMode: widget.time.mode,
      window,
      ctx: displayedCtx,
      gate: displayedGate,
    });
  }, [displayedCtx, displayedGate, declaration, runtime.client, following, widget.time.mode, window]);

  const projectionOptions = useMemo(
    () => (displayedCtx ? declaration.projectionOptions(displayedCtx) : ({} as TProjectionOptions)),
    [displayedCtx, declaration.projectionOptions],
  );

  const bounds = useMemo<readonly [number, number] | null>(
    () => (track ? [track.branch.firstHour, track.branch.lastHour] : null),
    [track?.branch.firstHour, track?.branch.lastHour],
  );

  const contextMenuItems = useCallback(
    (_key: string, _name: string, atHour: number) => (declaration.jumpToDate && track ? [jumpItem(runtime, track.id, atHour)] : []),
    [declaration.jumpToDate, runtime, track],
  );

  const customUnavailable = displayedCtx && declaration.unavailable ? declaration.unavailable(displayedCtx, staticSources?.hour ?? moment) : null;
  const combinedWarnings = useMemo(() => {
    const fromDecl = displayedCtx && declaration.warnings ? declaration.warnings(displayedCtx, followPlan.refused) : [];
    const fromGroups = groups.data?.warnings ?? [];
    const all = [...fromGroups, ...fromDecl, ...(extraWarnings ?? [])];
    return all.length ? all : undefined;
  }, [displayedCtx, declaration.warnings, followPlan.refused, groups.data?.warnings, extraWarnings]);

  const loadingLabel = `Loading ${declaration.subject}…`;
  const blockedState = (() => {
    if (!track) return <WidgetState kind="empty" icon={declaration.icon} message={declaration.emptyCampaign} />;
    if (isSingle) {
      if (!gate.open) {
        return <GateState
          gate={gate}
          loading={loadingLabel}
          choose={policy.choose ?? `Choose a country to see its ${declaration.subject}.`}
          absent={nameDetail ? 'This country does not exist at this date.' : `${countryName} does not exist at this date.`}
          detail={nameDetail}
        />;
      }
    } else if (coverage.outside) {
      return <WidgetState kind="outside" read={coverage} />;
    }
    if (followPlan.refused) return <WidgetState kind="unavailable" technical={followPlan.refused} />;
    if (auxEnabled && auxResult.error) return <WidgetState kind="error" failure={auxResult.error} onRetry={auxResult.retry} />;
    if (needsGroups && groups.error && !groups.data) return <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
    if (isFilter) {
      if (filterMatch?.reason) return <WidgetState kind="unavailable" message={filterMatch.reason} />;
      if (countriesState.reason) return <WidgetState kind="unavailable" message={countriesState.reason} />;
      if (!filterMatch?.ready || !groups.data) return <WidgetState kind="loading" label={loadingLabel} />;
    }
    if (isMulti) {
      if (needsSlots && slotState.error && !slots) {
        return <WidgetState kind="error" failure={slotState.error} onRetry={slotState.retry} />;
      }
      if ((needsGroups && !groups.data) || (needsSlots && !slots)) {
        return <WidgetState kind="loading" label={loadingLabel} />;
      }
      if (countriesState.reason) return <WidgetState kind="empty" message={countriesState.reason} />;
      if (effectiveMulti && !following) {
        const none = noMembersAt(widget.countries, effectiveMulti, groups.data, moment, lastHour);
        if (none) return <NoMembersState reason={none.reason} />;
        if (effectiveMulti.top == null && !countriesState.rankedAt(moment).length && !policy.allowSingleSlot) {
          const why = groups.data && widget.countries.kind === 'group' ? worldFilter(widget.countries, [], groups.data, moment, lastHour).reason : null;
          return <WidgetState kind="unavailable" message={why ?? 'No countries available for this selection.'} />;
        }
      }
    }
    if (auxEnabled && auxResult.data === undefined) return <WidgetState kind="loading" label={loadingLabel} />;
    if (customUnavailable) return <WidgetState kind="unavailable" message={customUnavailable} />;
    if (following) {
      if (followPlan.unavailable) return <WidgetState kind="unavailable" message={followPlan.unavailable} />;
      return null;
    }
    if (declaration.source.kind === 'head') {
      if (!window || !staticRange) return <WidgetState kind="outside" />;
    } else if (pinHour === null) {
      return <WidgetState kind="outside" />;
    }
    if (staticProduct.error) {
      return <WidgetState kind="error" what={declaration.subject} failure={staticProduct.error} onRetry={staticProduct.retry} />;
    }
    if (staticProduct.data === undefined) return <WidgetState kind="loading" label={loadingLabel} />;
    return null;
  })();

  const bannerFailure = (isSingle && singleCountry?.chosen && singleCountry.error && singleCountry.historyHeld)
    ? <WidgetState kind="error" scale="section" failure={singleCountry.error} onRetry={singleCountry.retry} />
    : ((isFilter || needsGroups) && groups.error && groups.data)
      ? <WidgetState kind="error" scale="section" failure={groups.error} onRetry={groups.retry} />
      : null;

  const host = <FamilyTableHost
    key={declaration.keyByBranch ? (track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id]) : 'none') : undefined}
    declaration={declaration.projection}
    options={projectionOptions}
    label={declaration.label}
    subject={declaration.subject}
    className={declaration.frameClassName}
    widget={widget}
    runtime={runtime}
    active={active}
    columns={columns}
    settings={settings}
    onSettings={setOption && declaration.settings.kind !== 'fixed' ? onSettings : undefined}
    editableColumns={declaration.settings.kind === 'table' || declaration.settings.kind === 'custom'}
    subscriptions={followPlan.subs}
    allowEmptyFollow={followPlan.allowEmptyFollow}
    staticSources={staticSources}
    exactSources={displayedCtx && perfLog && !blockedState ? exactSources : undefined}
    offset={track?.axisOffset ?? 0}
    pin={following ? null : (declaration.source.kind === 'panel' ? (staticSources?.hour ?? pinHour) : staticSources?.hour ?? null)}
    bounds={bounds}
    countryLooks={wantsColours ? labels : undefined}
    selectedKey={selectedKey}
    onActivate={onActivate}
    contextMenuItems={declaration.jumpToDate ? contextMenuItems : undefined}
    headerSlot={headerSlot}
    onDiagnostics={onDiagnostics}
    extraWarnings={combinedWarnings}
    blockedState={blockedState}
    held={staticHeld}
    loadingLabel={loadingLabel}
  />;

  if (!declaration.className && !bannerFailure) return host;
  return <div className={declaration.className} data-detail={isSingle ? countryLabel.nameFailure?.message : undefined}>
    {bannerFailure}
    {host}
  </div>;
}
