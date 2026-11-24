import { useCallback, useMemo, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import type { WidgetProps } from '../../dashboard/definitions';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { countryMatchFilter } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { contentSet, resolveOptions, toggleContentSet } from '../../dashboard/options';
import { loadProduct, useProduct } from '../../dashboard/products';
import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { forwardRequest, productRequest } from '../../dashboard/request.ts';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { coverageOf } from '../../dashboard/widgetTime';
import type { FamilySources } from '../../families/contract.ts';
import FamilyLogHost from '../../families/log/FamilyLogHost.tsx';
import type { LogControlsSpec, LogSourceSubscription } from '../../families/log/declaration.ts';
import { LOG_MIN_TAIL_LIMIT } from '../../families/log/layout.ts';
import { logFamilySettings } from '../../families/log/options.ts';
import { isLogTailReply, pageReplyToTail, scrollPageOrdinalRange, type LogTailReplyWire } from '../../families/log/tailReply.ts';
import { useLogLooks } from '../../families/log/useLogLooks.ts';
import type { DivisionLogManifestReply, LogPageReply } from '../../generated/dashboard';
import { OPTIONS } from './definition';
import {
  boundMembershipScope,
  divisionFacets,
  KINDS,
  selectedTemplates,
  shownKinds,
  templateToken,
  validateDivisionManifest,
  VANISHED_KIND,
  type CountryScopeFilter,
} from './divisionLogData.ts';
import { divisionLogProjection, type DivisionLogProjectionOptions } from './divisionLogProjection.ts';

const DIVISION_LOG_TAIL = 'division-log-tail';
const DIVISION_LOG_PAGE = 'division-log';

export default function DivisionLogWidget({ widget, runtime, active, onDiagnostics, setOption, headerSlot }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const gate = useGate(runtime, widget, SPEC, active);
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logFamilySettings(options);
  const follow = widget.time.mode === 'follow';
  const window = track && !follow ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null;
  const first = widget.time.mode === 'range' && window ? Math.floor(window.firstHour) : track?.branch.firstHour ?? 0;
  const last = track?.branch.lastHour ?? 0;
  const pinHour = follow ? null : (widget.time.mode === 'campaign' || !window ? last : Math.floor(window.lastHour));

  const manifest = useProduct<DivisionLogManifestReply>(
    runtime.client,
    track ? productRequest({
      sourceId: track.source.id,
      revision: track.source.revision,
      product: 'division-log-manifest',
      params: { branchId: track.branch.id },
    }) : null,
    active,
    validateDivisionManifest,
  );

  const campaign = track?.source.metadata.campaignKey ?? '';
  const branch = track?.branch.id ?? '';
  const templatesValue = typeof options.templates === 'string' ? options.templates : '';
  const templates = useMemo(() => selectedTemplates(templatesValue, campaign, branch), [templatesValue, campaign, branch]);
  const chosen = useMemo(() => contentSet(templatesValue), [templatesValue]);
  const templateChoices = useMemo(() => (manifest.data?.templates ?? []).map(template => ({
    value: templateToken(campaign, branch, template.id),
    label: template.name ?? `Unnamed template (${template.id})`,
  })), [manifest.data, campaign, branch]);

  const events = String(options.events);
  const needsGroups = widget.countries.kind === 'group' || widget.countries.kind === 'factions';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const countryScope = useMemo<CountryScopeFilter | null | undefined>(() => {
    if (widget.countries.kind === 'all') return null;
    if (!needsGroups) return { kind: 'countries', tags: selectedCountryKeys(widget.countries, track) };
    if (!groups.data) return undefined;
    const filter = countryMatchFilter(widget.countries, [], groups.data, last, undefined, track?.branch.firstHour);
    return boundMembershipScope(filter.membership ?? [], filter.unobserved, gate.span);
  }, [widget.countries, needsGroups, track, groups.data, last, gate.span]);
  const waiting = countryScope === undefined && widget.countries.kind !== 'all';
  const emptyScope = countryScope !== null && countryScope !== undefined && (
    countryScope.kind === 'countries'
      ? countryScope.tags.length === 0
      : countryScope.membership.length === 0 && countryScope.unobserved.length === 0
  );

  const kinds = useMemo(() => shownKinds(events), [events]);
  const losses = manifest.data?.lossesAvailable === true;
  const facets = useMemo(
    () => divisionFacets(losses ? kinds : kinds.filter(kind => kind === 'created'), templates, emptyScope ? null : (countryScope ?? null)),
    [kinds, losses, templates, emptyScope, countryScope],
  );
  const showTotals = options.totals === true;
  const countFacets = useMemo(() => {
    const rest = facets.slice(1);
    const asks: { key: string; facets: typeof facets }[] = [];
    if (showTotals) {
      for (const kind of facets[0]!.values) {
        const key = kind === KINDS.created ? 'created' : kind === KINDS.lost ? 'lost' : 'transfer';
        asks.push({ key, facets: [{ name: 'kind', values: [kind], countries: [] }, ...rest] });
      }
    }
    asks.push({ key: 'vanished', facets: [{ name: 'kind', values: [VANISHED_KIND], countries: [] }, ...rest] });
    return asks;
  }, [showTotals, facets]);

  const [limit, setLimit] = useState(LOG_MIN_TAIL_LIMIT);
  const logSpan = gate.span(first, last);
  const fromParam = widget.time.mode === 'range' && logSpan ? logSpan.firstHour : null;
  const looks = useLogLooks(runtime, track, track?.branch.firstHour ?? 0, last, active && !!manifest.data?.available);

  const tailParams = useMemo(() => {
    if (!track || !manifest.data?.available || waiting || emptyScope || !facets[0]?.values.length) return null;
    return {
      branchId: track.branch.id,
      facets,
      ...(fromParam !== null ? { from: fromParam } : {}),
      limit,
      countFacets,
    };
  }, [track, manifest.data?.available, waiting, facets, fromParam, limit, countFacets]);

  const followPlan = useMemo(() => {
    if (!follow || !track || !tailParams || !active) {
      return { subs: [] as LogSourceSubscription[], refused: null as string | null };
    }
    const checked = gate.checkFollow({ panel: DIVISION_LOG_TAIL, ...tailParams });
    if (checked.refused) return { subs: [], refused: checked.refused };
    if (!checked.query) return { subs: [], refused: null };
    return {
      subs: [{
        name: 'tail',
        source: track.source,
        branch: track.branch.id,
        offset: track.axisOffset,
        query: checked.query,
        channel: 'panel' as const,
        product: DIVISION_LOG_TAIL,
      }],
      refused: null,
    };
  }, [follow, track, tailParams, active, gate.checkFollow]);

  const staticReq = useMemo(() => {
    if (follow || !track || !tailParams || pinHour === null) return null;
    const clamped = gate.recorded(pinHour);
    if (clamped === null) return null;
    return forwardRequest(track.source, DIVISION_LOG_TAIL, { ...tailParams, hour: clamped });
  }, [follow, track, tailParams, pinHour, gate.recorded]);

  const staticReply = useProduct<LogTailReplyWire>(
    runtime.client,
    staticReq,
    active && !follow && !!staticReq,
    value => {
      if (!isLogTailReply(value)) throw new Error('The division log tail reply is malformed.');
      return value;
    },
    track?.source,
  );

  const staticSources = useMemo(() => {
    if (follow || !staticReply.data || pinHour === null) return null;
    const panels = new Map<string, unknown>([['tail', staticReply.data]]);
    const sources: FamilySources = { answers: new Map(), panels, live: new Map() };
    return { sources, hour: pinHour };
  }, [follow, staticReply.data, pinHour]);

  const exactSources = useCallback(async (hour: number, askLimit: number): Promise<FamilySources | null> => {
    if (!track || !tailParams) return null;
    const clamped = gate.recorded(hour);
    if (clamped === null) return null;
    const controller = new AbortController();
    const reply = await loadProduct<LogTailReplyWire>(
      runtime.client,
      forwardRequest(track.source, DIVISION_LOG_TAIL, { ...tailParams, hour: clamped, limit: askLimit }),
      controller.signal,
      value => {
        if (!isLogTailReply(value)) throw new Error('The division log tail reply is malformed.');
        return value;
      },
      track.source,
    );
    return { answers: new Map(), panels: new Map<string, unknown>([['tail', reply]]), live: new Map() };
  }, [track, tailParams, gate.recorded, runtime.client]);

  const onLoadScrollPage = useCallback(async (frozenHour: number, offset: number, pageSize: number, baseOrdinal: number, total: number) => {
    if (!track || !tailParams) return null;
    const range = scrollPageOrdinalRange(baseOrdinal, total, offset, pageSize);
    if (!range) return null;
    const clamped = gate.recorded(frozenHour);
    if (clamped === null) return null;
    const controller = new AbortController();
    const page = await loadProduct<LogPageReply>(
      runtime.client,
      forwardRequest(track.source, DIVISION_LOG_PAGE, {
        branchId: track.branch.id,
        facets,
        ...(fromParam !== null ? { from: fromParam } : {}),
        to: clamped,
        cursor: range.cursor,
        pageSize: range.pageSize,
      }),
      controller.signal,
      undefined,
      track.source,
    );
    return pageReplyToTail(page, baseOrdinal, total);
  }, [track, tailParams, gate.recorded, runtime.client, facets, fromParam]);

  const projectionOptions = useMemo<DivisionLogProjectionOptions>(() => ({
    events,
    showTotals,
    lossesAvailable: losses,
    fromHour: fromParam ?? track?.branch.firstHour,
    looks,
    manifestWarnings: manifest.data?.warnings ?? [],
    manifestGaps: manifest.data?.gaps ?? [],
  }), [events, showTotals, losses, fromParam, track?.branch.firstHour, looks, manifest.data?.warnings, manifest.data?.gaps]);

  const controls = useMemo<LogControlsSpec | null>(() => {
    if (!log.chips) return null;
    return {
      chipGroups: [{
        key: 'events',
        label: 'Division events',
        chips: [
          { value: 'both', label: 'Both' },
          { value: 'created', label: 'Created' },
          { value: 'lost', label: 'Lost / transferred' },
        ],
        selected: [events],
        max: 1,
        full: false,
        onToggle: value => setOption?.('events', value),
      }],
      pickers: chosen.length > 0 ? [{
        key: 'templates',
        label: 'Division template types',
        automatic: 'All templates',
        choices: templateChoices,
        selected: chosen,
        onReset: () => setOption?.('templates', ''),
        onToggle: value => {
          const next = toggleContentSet(templatesValue, value, Number.MAX_SAFE_INTEGER);
          if (next !== null) setOption?.('templates', next);
        },
      }] : [],
    };
  }, [log.chips, events, setOption, chosen, templateChoices, templatesValue]);

  const contextMenuItems = useCallback((_key: string, hour: number) => (track ? [jumpItem(runtime, track.id, hour)] : []), [runtime, track]);

  if (!track) return <WidgetState kind="empty" icon="shield" message="Load a campaign to see its divisions." />;

  const blockedState = gate.outside ? <WidgetState kind="outside" read={coverageOf(gate.outside)} />
    : manifest.error ? <WidgetState kind="error" failure={manifest.error} onRetry={manifest.retry} />
    : needsGroups && groups.error && !groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />
    : !manifest.data || waiting ? <WidgetState kind="loading" label="Loading division log…" />
    : emptyScope ? <WidgetState kind="unavailable" message={needsGroups ? 'This group has no members in this campaign.' : 'Choose a country in the widget settings.'} />
    : !manifest.data.available || (events === 'lost' && !losses) ? <WidgetState kind="unavailable" message="Not in this recording" />
    : followPlan.refused ? <WidgetState kind="unavailable" message={followPlan.refused} />
    : !follow && staticReply.error ? <WidgetState kind="error" failure={staticReply.error} onRetry={staticReply.retry} />
    : null;

  const bounds = logSpan ? [logSpan.firstHour, logSpan.lastHour] as const : null;

  return (
    <FamilyLogHost
      projection={divisionLogProjection}
      options={projectionOptions}
      label="Division log"
      subject="divisions"
      widget={widget}
      runtime={runtime}
      active={active}
      density={log.density}
      months={log.months}
      subscriptions={followPlan.subs}
      staticSources={staticSources}
      exactSources={exactSources}
      onLoadScrollPage={onLoadScrollPage}
      onLimitChange={setLimit}
      offset={follow ? track.axisOffset : 0}
      pin={pinHour}
      bounds={bounds}
      controls={controls}
      gapsSubject="Division events are not recorded whole"
      note={!losses && events !== 'created' ? 'Losses and transfers: Not in this recording.' : null}
      contextMenuItems={contextMenuItems}
      headerSlot={headerSlot}
      onDiagnostics={onDiagnostics}
      blockedState={blockedState}
    />
  );
}
