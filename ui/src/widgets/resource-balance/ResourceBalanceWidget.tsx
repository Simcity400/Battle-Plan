import { useInstalledValue } from '../../dashboard/follow/useFollowed.ts';
import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { keyNameAt, useSlotTags } from '../../dashboard/countrySlots.ts';
import { useCallback, useEffect, useId, useLayoutEffect, useMemo,  useRef, useState } from 'react';
import { setAttr, setStyle, setText } from '../../charts/domText';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { Select } from '../../components/Select';
import '../../components/ChipSet.css';
import { useContextMenu } from '../../components/ContextMenu';
import DrillHost from '../../charts/drill/DrillHost';
import {  pinnedBreakdown } from '../../charts/drill/drillState';
import { useDrill } from '../../charts/drill/useDrill';
import { dayLabel } from '../../charts/draw';
import { pickSummaryLines } from '../../charts/chartSummary';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import type { Live, LiveValues } from '../../charts/live';
import { SECONDARY_DASH } from '../../charts/palette';
import TimeChart from '../../charts/TimeChart';
import type { ChartData, ChartLayer } from '../../charts/TimeChart';
import { tooltipText } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import type { ChartTokens } from '../../charts/tokens';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { formatSigned } from '../../charts/valueAxis';
import { easeTau, motionOf } from '../../charts/easing.ts';
import type { CSSProperties } from 'react';
import CountryComparisons from '../../dashboard/CountryComparisons';
import type { ComparisonProps } from '../../dashboard/CountryComparisons';
import DatedCountryChip from '../../dashboard/DatedCountryChip';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { readDatedCountry } from '../../dashboard/datedCountry';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { contentKey, contentSet, resolveOptions, setToken } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { keyId } from '../../dashboard/keyedSeriesData';
import { useKeyedSeries } from '../../dashboard/keyedSeries';
import { usePlaybackHour } from '../../dashboard/follow/useFollow';
import { repliedCells, reporting } from '../../dashboard/follow/liveCells';
import { useShownHour } from '../../dashboard/widgetPerf';
import { BREAKDOWN, OPTIONS } from './definition';
import { RESOURCE_BALANCE, RESOURCE_SIZES, automaticResource, balanceOf, barParts, barStructure, campaignScale, chosenResources, dealsOf, momentScale,
  orderResources, overTimeTooltip, partWidth, poolsAt, poolsMissing, poolsOf, resourceName, resourceTimeLayers, resourceTooltip, toggleResource, validateResourceBalance,
  validateResourceSizes, describeResource, legendFlags, segmentLook } from './resourceData';
import type { Balance, LineAt, RowOrder } from './resourceData';
import ResourceBreakdown from './ResourceBreakdown';
import './resourceBalance.css';
import type { ResourceBalanceReply, ResourceInfo } from '../../generated/dashboard';
import { regionFollows, SPEC, type GatedHour } from '../../dashboard/regionTime.ts';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';
import { UNRECORDED_MESSAGE } from '../../dashboard/regionTime.ts';

const NO_WARNINGS: readonly string[] = [];
const HOLDERS = { holders: true };
const SUPPLY = { kind: RESOURCE_BALANCE, measure: 'supply' } as const;
const DEMAND = { kind: RESOURCE_BALANCE, measure: 'demand' } as const;

export default function ResourceBalanceWidget(props: WidgetProps) {
  return props.widget.options?.show === BREAKDOWN.id ? <ResourceCountry {...props} /> : <CountryComparisons {...props} component={ResourceCountry} />;
}

const isMember = (within: ComparisonProps['within'], hour: number | null) =>
  !within || hour === null || within.some(span => span.firstHour <= hour && hour <= span.lastHour);

function savedCountryValid(widget: WidgetSpec, track: BoundTrack): boolean {
  const stored = resolveOptions(OPTIONS, widget.options).resourceCountry;
  return widget.countries.kind === 'tags' && widget.countries.tags.length === 1
    && (!stored || readDatedCountry(stored, track.source.metadata.campaignKey, track.branch.id, widget.countries) !== null);
}

function ResourceCountry(props: ComparisonProps) {
  const { widget, runtime } = props;
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const saved = options.show === BREAKDOWN.id;
  const tag = selectedCountryKeys(widget.countries, track)[0] ?? null;
  if (!track) return <WidgetState kind="empty" icon="layers" message="Load a campaign to see resources." />;
  if (saved && !savedCountryValid(widget, track)) return <WidgetState kind="unavailable" message="Saved country selection unavailable. Choose it again." />;
  if (!tag) return <WidgetState kind="empty" message="Choose a country in widget settings." />;
  return <ResourceCountryView {...props} track={track} tag={tag} saved={saved} />;
}

function partColours(tokens: ChartTokens): Map<string, string> {
  const [supply, demand] = [tokens.categories[0], tokens.categories[1]];
  return new Map([
    ['produced', supply], ['subjects', `color-mix(in srgb, ${supply} 75%, ${tokens.surface})`], ['imported', `color-mix(in srgb, ${supply} 50%, ${tokens.surface})`],
    ['needed', demand], ['overlord', `color-mix(in srgb, ${demand} 75%, ${tokens.surface})`], ['exported', `color-mix(in srgb, ${demand} 50%, ${tokens.surface})`],
  ]);
}

function ResourceCountryView(props: ComparisonProps & { track: BoundTrack; tag: string; saved: boolean }) {
  const { widget, runtime, active, countrySlot, track, tag, saved, setOption } = props;
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const menu = useContextMenu('Resource actions');
  const anchor = useRef<HTMLDivElement>(null);
  const needs = useMemo(() => saved ? { country: { stored: options.resourceCountry, key: 'resourceCountry', setOption }, holders: true } : HOLDERS,
    [saved, options.resourceCountry, setOption]);
  const drillControl = useDrill(runtime, useRegion(widget), saved ? BREAKDOWN.id : null, undefined, undefined, needs);
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const [resourceId, setResourceId] = useState<number>();
  const following = widget.time.mode === 'follow';
  const requestHour = usePlaybackHour(runtime.clock, active && !following && widget.time.mode !== 'campaign');
  const window = resolveWindow(widget, track, runtime.tracks, requestHour);
  const savedSelection = saved ? readDatedCountry(options.resourceCountry, track.source.metadata.campaignKey, track.branch.id, widget.countries) : null;
  const savedExtent = savedSelection?.extent ? [track.branch.firstHour + savedSelection.extent[0], track.branch.firstHour + savedSelection.extent[1]] as [number, number] : null;
  const extent: [number, number] = savedExtent && savedExtent[0] >= track.branch.firstHour && savedExtent[1] <= track.branch.lastHour ? savedExtent
    : widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour];
  const playhead = window && (following || widget.time.mode === 'pin') ? Math.floor(window.lastHour) : null;
  useEffect(() => { dispatch({ type: 'close' }); setResourceId(undefined); }, [track.source.id, track.source.revision, track.branch.id, tag, options.show]);

  const shown = !!saved || drill.breakdown !== null;
  const drillFollows = regionFollows(drillControl.region);
  const drillAt = drillControl.hour;
  const drillTag = drillControl.gate.country?.tag ?? null;
  const breakdown = useHeadProduct(runtime, drillControl.gate, RESOURCE_BALANCE, shown && drillTag ? { country: countryFromKey(drillTag) } : null,
    { active: active && shown, validate: validateResourceBalance });
  const drillData = breakdown.data;
  const savedKey = contentKey(String(options.partResource ?? ''));
  const picked = !saved ? drillData?.resources.find(resource => resource.id === resourceId)
    : savedKey ? drillData?.resources.find(resource => JSON.stringify([resource.token, resource.ordinal]) === savedKey) : drillData?.resources[0];
  const dayAt = drillFollows ? breakdown.at : drillAt;
  const header = useCountryLabelsAt(runtime.client, track, saved && countrySlot && dayAt !== null && drillTag ? [{ tag: drillTag, hour: dayAt }] : [], active);
  const pinnedHeader = saved && countrySlot ? createPortal(<DatedCountryChip failure={breakdown.error} hour={dayAt} reason={null}
    label={dayAt !== null && drillTag ? header.labelAt(dayAt)(drillTag) : null}
    live={drillFollows ? { read: () => breakdown.shownAt(), subscribe: changed => breakdown.live.subscribe(changed) } : undefined} />, countrySlot) : null;
  const axisOffset = track.axisOffset ?? 0;
  const liveHead = useMemo(() => following ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [following, runtime.clock, axisOffset]);
  const drillLiveHead = useMemo(() => drillFollows ? { read: () => runtime.clock.getSnapshot().position + axisOffset, subscribe: runtime.clock.subscribeFrame } : undefined,
    [drillFollows, runtime.clock, axisOffset]);

  const open = (resource: ResourceInfo, hour: number | null) => {
    setResourceId(resource.id);
    const at = hour ?? Math.floor(runtime.clock.getHour() + axisOffset);
    dispatch({ type: 'open', breakdown: BREAKDOWN.id, hour: at, follow: drillControl.followsAt(at), country: { tag, origin: at } });
  };
  const jump = (hour: number) => [jumpItem(runtime, track.id, hour)];

  const stage = drillControl.gate.stage;
  const detail = breakdown.outside ? <WidgetState kind="outside" read={breakdown} />
    : saved && stage.stage === 'failed' ? <WidgetState kind="error" failure={stage.failure} onRetry={stage.retry} />
    : saved && stage.stage === 'holders' ? <WidgetState kind="unavailable" message="More than one country held this tag. Choose the country again in settings." />
    : saved && stage.stage === 'absent' ? <WidgetState kind="unavailable" message={stage.reason ?? 'The chosen country did not exist at this time.'} />
    : breakdown.error ? <WidgetState kind="error" failure={breakdown.error} onRetry={breakdown.retry} />
      : !drillData ? <WidgetState kind="loading" label="Loading resources…" read={breakdown} />
        : drillData.poolsMissing ? <WidgetState kind="unavailable" message={drillData.poolsMissing} />
          : !picked ? <p className="drill-empty">{saved && !savedKey ? 'No resource was recorded at this time.' : 'Resource unavailable. Choose another in settings.'}</p>
            : <ResourceBreakdown runtime={runtime} gate={drillControl.gate} track={track} active={active} history={drillControl.gate.country?.history ?? null} resource={picked} data={drillData} live={breakdown.live}
              hour={dayAt} following={drillFollows} tokens={tokens} extent={extent} playhead={drillFollows ? null : drillAt} liveHead={drillLiveHead}
              playback={(options.playback as PlaybackMode) ?? 'draw-in'} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
              hoursPerSecond={motionOf(runtime.clock.getSnapshot())}
              onPick={hour => dispatch({ type: 'pick', hour })}
              onContextMenu={(hour, x, y) => menu.open(x, y, jump(hour))} />;

  if (saved) return <div ref={anchor} className="resource-balance">
    <div className="drill-body" ref={breakdown.root}>
      {pinnedHeader}
      {!drillFollows && <button type="button" className="button small quiet" onClick={drillControl.follow}>Follow playback</button>}
      {detail}
    </div>
    {menu.menu}
  </div>;

  const pinTimed = drillControl.gate.checkTime();
  const pinTime = pinTimed.time;
  const pinExtent = drillControl.gate.span(extent[0], extent[1]);
  const country = drillControl.hold(drillControl.gate.country?.holder, pinExtent);
  const main = options.view === 'time'
    ? <ResourcesOverTime {...props} extent={extent} playhead={playhead} liveHead={liveHead} onPick={open} />
    : <ResourceBars {...props} onPick={open} />;
  return <div ref={anchor} className="resource-balance-host">
    <DrillHost drill={drillControl} runtime={runtime} anchor={anchor} mode={options.drill === 'popout' ? 'popout' : 'inline'}
      title={picked ? resourceName(picked) : 'Resource breakdown'} day={dayAt === null ? '' : dayLabel(dayAt)} canFollow
      onClose={() => dispatch({ type: 'close' })} main={main}
      pinRefused={pinTimed.refused ?? null} onPin={picked && pinTime && pinExtent && country ? () => {
        const pinned = pinnedBreakdown({ ...widget, countries: { kind: 'tags', tags: [country.tag] } }, { ...BREAKDOWN, label: resourceName(picked) }, track.id, pinTime);
        pinned.options = { ...pinned.options, partResource: setToken(JSON.stringify([picked.token, picked.ordinal])), resourceCountry: country.token };
        runtime.addWidgetBelow(widget.id, pinned);
      } : undefined}>
      <div ref={breakdown.root}>
        <div className="select-row">Resource <Select label="Resource" value={String(picked?.id ?? '')} onChange={value => setResourceId(Number(value))}
          options={(drillData?.resources ?? []).map(resource => ({ value: String(resource.id), label: resourceName(resource) }))} /></div>
        {detail}
      </div>
    </DrillHost>
    {menu.menu}
  </div>;
}

interface RowCells { parts: Map<string, HTMLElement>; net: HTMLElement | null; row?: HTMLElement | null; resource?: ResourceInfo; description?: HTMLElement | null }

function ResourceBars({ widget, runtime, active, onDiagnostics, within, tag, track, onPick }: ComparisonProps & {
  tag: string; track: BoundTrack; onPick: (resource: ResourceInfo, hour: number | null) => void;
}) {
  const options = resolveOptions(OPTIONS, widget.options);
  const order = (options.order as RowOrder) ?? 'game';
  const hideEmpty = options.hideEmpty === true;
  const campaign = options.scale !== 'moment';
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const menu = useContextMenu('Resource actions');
  const owner = useId();
  const sizes = useProduct(runtime.client, campaign ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: RESOURCE_SIZES,
    params: { branchId: track.branch.id, country: countryFromKey(tag) } }) : null, active, validateResourceSizes, track.source);
  const colours = useMemo(() => partColours(tokens), [tokens]);
  const cells = useRef(new Map<number, RowCells>());
  const head = useHeadProduct(runtime, useGate(runtime, widget), RESOURCE_BALANCE, { country: countryFromKey(tag), order, ...hideEmpty ? { hideEmpty } : {} },
    { active, validate: validateResourceBalance, structure: barStructure });
  const bars = useRef<HTMLOListElement | null>(null);
  const described = useRef<(resource: ResourceInfo) => TooltipContent | null>(() => null);
  const data = head.data;
  const report = useShownHour();
  const shownAt = useRef(head.shownAt);
  shownAt.current = head.shownAt;
  const live = useMemo(() => reporting(head.live, () => report(shownAt.current())), [head.live, report]);
  const scaleOf = useCallback((reply: ResourceBalanceReply, values: LiveValues<number>) => {
    const whole = campaign ? campaignScale(sizes.data) : 0n, moment = momentScale(poolsOf(reply, values));
    return whole > moment ? whole : moment;
  }, [campaign, sizes.data]);
  useLayoutEffect(() => {
    if (!data) return undefined;
    const paint = () => {
      const values = live.read(), scale = scaleOf(data, values);
      for (const [id, held] of cells.current) {
        const pools = poolsAt(values, id);
        const balance = pools && balanceOf(pools), parts = balance ? barParts(balance) : null;
        for (const [part, raw] of parts ? [...parts.supply, ...parts.demand] : []) {
          const element = held.parts.get(part);
          setStyle(element, 'width', partWidth(raw, scale));
          for (const [property, value] of Object.entries(segmentLook(raw, colours.get(part)))) setStyle(element, property, value);
        }
        if (held.resource) setAttr(held.row, 'aria-label', describeResource(held.resource, balance, data.decimals));
        if (held.resource) setText(held.description, tooltipText(described.current(held.resource) ?? { title: resourceName(held.resource), rows: [] }));
        if (!held.parts.size || !held.net) continue;
        if (!balance) for (const element of held.parts.values()) setStyle(element, 'width', '0%');
        setText(held.net, balance ? formatSigned(balance.net, data.decimals) : '—');
        setAttr(held.net, 'class', !balance ? 'is-missing' : balance.net < 0n ? 'is-negative' : 'is-positive');
        setAttr(held.net, 'title', balance ? '' : poolsMissing(values, id));
      }
    };
    const label = () => { const at = head.atNow(); setAttr(bars.current, 'aria-label', at === null ? 'Resources' : `Resources on ${dayLabel(at)}`); };
    paint(); label();
    return live.subscribe(() => { paint(); label(); });
  }, [data, live, scaleOf, colours, head.atNow]);
  const flags = useInstalledValue(data ? live.subscribe : undefined, () => data
    ? legendFlags(data.pools.map(id => { const pools = poolsAt(live.read(), id); return pools ? balanceOf(pools) : null; })) : 'false|false');
  useDiagnostics(onDiagnostics, data ? [...data.warnings, ...(sizes.data?.warnings ?? [])] : NO_WARNINGS);
  const hour = head.at ?? Math.floor(runtime.clock.getHour());
  const partners = useMemo(() => [...new Set(data?.deals.map(deal => deal.partner) ?? [])].map(partner => ({ tag: partner, hour })), [data?.deals, hour]);
  const labels = useCountryLabelsAt(runtime.client, track, partners, active);
  const breakdown = useCallback((resource: ResourceInfo): TooltipContent | null => {
    const values = live.read();
    const pools = data ? poolsAt(values, resource.id) : null;
    if (!data || !pools) return { title: resourceName(resource), rows: [{ label: data?.poolsMissing ?? (data ? poolsMissing(values, resource.id) : 'Not recorded at this time'), tone: 'muted' }] };
    const at = head.atNow() ?? hour;
    return resourceTooltip({ title: resourceName(resource), icon: resource.icon ?? undefined, subtitle: `Per day · ${dayLabel(at)}` },
      balanceOf(pools), dealsOf(data, resource.id, values), data.decimals, labels.labelAt(at), viewport);
  }, [data, live, labels, hour, viewport, head.atNow]);
  described.current = breakdown;
  const rowTips = useRowTooltips<number>({ owner, active, revision: `${hour}:${data ? barStructure(data) : ''}:${labels.revision}`, refresh: live.subscribe,
    content: id => { const resource = data?.resources.find(item => item.id === id); return resource ? breakdown(resource) : null; },
    onMenu: (_, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)]) });

  let body;
  if (head.error) body = <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
  else if (head.read === 'off' && !data) body = <WidgetState kind="outside" />;
  else if (!data) body = <WidgetState kind="loading" label="Loading resources…" />;
  else if (!isMember(within, head.at)) body = <WidgetState kind="unavailable" message="This country is not in the group at this date." />;
  else if (data.poolsMissing) body = <WidgetState kind="unavailable" message={data.poolsMissing} />;
  else {
    const values = live.read();
    const held = new Set(data.pools);
    const rows = orderResources(data).map(resource => {
      const pools = held.has(resource.id) ? poolsAt(values, resource.id) : null;
      return { resource, held: held.has(resource.id), balance: pools ? balanceOf(pools) : null };
    });
    const scale = scaleOf(data, values);
    const [subjects, overlord] = flags.split('|').map(flag => flag === 'true');
    const legend = [
      { id: 'produced', label: 'Produced', color: colours.get('produced')! },
      ...(subjects ? [{ id: 'subjects', label: 'From subjects', color: colours.get('subjects')! }] : []),
      { id: 'imported', label: 'Imported', color: colours.get('imported')! },
      { id: 'needed', label: 'Needed', color: colours.get('needed')! },
      ...(overlord ? [{ id: 'overlord', label: 'To the overlord', color: colours.get('overlord')! }] : []),
      { id: 'exported', label: 'Exported', color: colours.get('exported')! },
    ];
    const cellsOf = (id: number) => {
      const entry = cells.current.get(id) ?? { parts: new Map<string, HTMLElement>(), net: null };
      cells.current.set(id, entry);
      return entry;
    };
    const zero = { produced: 0n, fromSubjects: 0n, toOverlord: 0n, imported: 0n, exportReserve: 0n, exported: 0n, available: 0n, projects: 0n, production: 0n,
      supply: 0n, demand: 0n, net: 0n } satisfies Balance;
    const segments = (id: number, parts: readonly [string, bigint][]) => parts.map(([part, raw], index) =>
      <i key={part} ref={element => { if (element) cellsOf(id).parts.set(part, element); else cellsOf(id).parts.delete(part); }}
        style={{ width: partWidth(raw, scale), marginLeft: index > 0 ? 1 : 0, ...(raw < 0n ? { boxShadow: `inset 0 0 0 1.5px ${colours.get(part)}` } : { background: colours.get(part) }) }} />);
    const describe = (resource: ResourceInfo, balance: Balance | null) => describeResource(resource, balance, data.decimals);
    body = <div className="resource-balance">
      <p className="resource-caption">Per day · outlined parts are negative{data.dealsMissing ? ` · ${data.dealsMissing}` : ''}</p>
      <Legend label="Bar parts" items={legend} />
      <ol className="resource-bars" ref={bars}
        style={{ '--resource-glide': `${Math.round(3 * easeTau(tokens.motion, motionOf(runtime.clock.getSnapshot()), tokens.reducedMotion))}ms` } as CSSProperties}>
        {rows.map(({ resource, held: has, balance }) => <li key={resource.id} role="button" aria-label={describe(resource, balance)} {...rowTips.rowProps(resource.id)}
          ref={element => { const held = cellsOf(resource.id); held.row = element; held.resource = resource; }}
          onClick={() => onPick(resource, head.hour)} onKeyDown={event => {
            if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); onPick(resource, head.hour); }
            else rowTips.rowProps(resource.id).onKeyDown(event);
          }}>
          <span id={rowTips.descriptionId(resource.id)} ref={element => { cellsOf(resource.id).description = element; }} className="sr-only">{tooltipText(breakdown(resource) ?? { title: resourceName(resource), rows: [] })}</span>
          <span className="resource-name">{resource.icon ? <img src={resource.icon} alt="" /> : null}<span>{resourceName(resource)}</span></span>
          {has ? <>
            <span className="resource-track" aria-hidden="true">
              <span className="resource-bar">{segments(resource.id, barParts(balance ?? zero).supply)}</span>
              <span className="resource-bar">{segments(resource.id, barParts(balance ?? zero).demand)}</span>
            </span>
            <b ref={element => { cellsOf(resource.id).net = element; }} className={!balance ? 'is-missing' : balance.net < 0n ? 'is-negative' : 'is-positive'}>
              {balance ? formatSigned(balance.net, data.decimals) : '—'}</b>
          </> : <span className="resource-missing">Not recorded</span>}
        </li>)}
      </ol>
      {menu.menu}
    </div>;
  }
  return <div ref={head.root} className="resource-balance-host">{body}</div>;
}

function ResourcesOverTime({ widget, runtime, active, onDiagnostics, within, tag, track, extent, playhead, liveHead, setOption, onPick }: ComparisonProps & {
  tag: string; track: BoundTrack; extent: [number, number]; playhead: number | null; liveHead?: { read(): number; subscribe(changed: () => void): () => void };
  onPick: (resource: ResourceInfo, hour: number | null) => void;
}) {
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const menu = useContextMenu('Resource actions');
  const following = widget.time.mode === 'follow';
  const [hoverHour, setHoverHour] = useState<number | null>(null);
  const [rootElement, setRoot] = useState<HTMLDivElement | null>(null);
  const root = useMemo(() => ({ current: rootElement }), [rootElement]);
  const gate = useGate(runtime, widget, SPEC, active);
  const atHead = useHeadProduct(runtime, gate, RESOURCE_BALANCE, { country: countryFromKey(tag) }, { active, validate: validateResourceBalance,
    structure: reply => JSON.stringify([reply.resources, reply.pools, reply.poolsMissing]) });
  const request = (hour: GatedHour) => (productRequest({ sourceId: track.source.id, revision: track.source.revision, product: RESOURCE_BALANCE, params: { branchId: track.branch.id, country: countryFromKey(tag), hour } }));
  const sized = gate.span(extent[0], extent[1]);
  const sizes = useProduct(runtime.client, sized ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: RESOURCE_SIZES,
    params: { branchId: track.branch.id, country: countryFromKey(tag), from: sized.firstHour, to: sized.lastHour } }) : null, active, validateResourceSizes, track.source);
  const hoverAt = gate.recorded(hoverHour, within);
  const hovered = useProduct(runtime.client, hoverAt === null ? null : request(hoverAt), active, validateResourceBalance, track.source);
  useDiagnostics(onDiagnostics, atHead.data ? [...atHead.data.warnings, ...(sizes.data?.warnings ?? [])] : NO_WARNINGS);
  const resources = useMemo(() => atHead.data?.resources ?? [], [atHead.data]);
  const storedText = typeof options.resources === 'string' ? options.resources : '';
  const stored = useMemo(() => contentSet(storedText), [storedText]);
  const automatic = useMemo(() => automaticResource(resources, sizes.data), [resources, sizes.data]);
  const chosen = useMemo(() => chosenResources(resources, stored, automatic), [resources, stored, automatic]);
  const chosenInfo = useMemo(() => chosen.flatMap(token => resources.filter(item => item.token === token)), [chosen, resources]);
  const keys = useMemo(() => chosenInfo.map(resource => ({ tag, part: String(resource.id) })), [chosenInfo, tag]);
  const membership = useMemo(() => within ? new Map(keys.map(key => [keyId(key), within])) : undefined, [keys, within]);
  const spec = (family: typeof SUPPLY | typeof DEMAND) => keys.length ? { source: track.source, branch: track.branch, family, keys, partBits: 32,
    within: membership, lastHour: track.branch.lastHour, axisOffset: track.axisOffset } : null;
  const head = following ? null : gate.hour;
  const supply = useKeyedSeries(runtime.client, runtime.clock, gate, spec(SUPPLY), head, hoverAt, following, active, root);
  const demand = useKeyedSeries(runtime.client, runtime.clock, gate, spec(DEMAND), head, hoverAt, following, active, root);
  const lines = useMemo(() => chosenInfo.map((resource, index) => ({ resource, key: keys[index], color: tokens.categories[index % tokens.categories.length] })),
    [chosenInfo, keys, tokens.categories]);
  const partners = useMemo(() => hoverHour === null ? [] : [...new Set(hovered.data?.deals.map(deal => deal.partner) ?? [])].map(partner => ({ tag: partner, hour: Math.floor(hoverHour) })),
    [hovered.data, hoverHour]);
  const labels = useCountryLabelsAt(runtime.client, track, partners, active);
  const slots = useSlotTags(runtime.client, track, active && isSlotKey(tag));
  const name = keyNameAt(slots, tag, playhead ?? extent[1]);
  const playback = (options.playback as PlaybackMode) ?? 'draw-in';
  const until = shownUntil(playhead, playback, options.afterPlayhead === true);
  const shade = lines.length === 1 && options.gap !== false;
  const layers = useMemo<ChartLayer[]>(() => resourceTimeLayers(
    lines.map(line => ({ id: keyId(line.key), color: line.color })),
    shade, supply.summaryOf, demand.summaryOf,
    () => liveHead && playhead !== null ? shownUntil(liveHead.read(), playback, options.afterPlayhead === true) : until,
    { above: tokens.positive, below: tokens.error },
  ), [lines, shade, supply.summaryOf, demand.summaryOf, liveHead, playhead, playback, options.afterPlayhead, until, tokens.positive, tokens.error]);
  const chartData = useMemo<ChartData>(() => ({
    revision: () => [supply.revision(), demand.revision()],
    subscribe: listener => { const stopSupply = supply.subscribe(listener), stopDemand = demand.subscribe(listener); return () => { stopSupply(); stopDemand(); }; },
  }), [supply.revision, supply.subscribe, demand.revision, demand.subscribe]);
  const [selection, setSelection] = useState<{ hour: number; ids: readonly string[] } | null>(null);
  useEffect(() => setSelection(null), [chosen, extent[0], extent[1]]);
  const chipValues = useRef(new Map<number, HTMLElement>());
  const report = useShownHour();
  const shownAt = useRef(atHead.shownAt);
  shownAt.current = atHead.shownAt;
  useLayoutEffect(() => {
    const data = atHead.data;
    if (!data) return undefined;
    const paint = () => {
      const values = atHead.live.read();
      report(shownAt.current());
      for (const [id, element] of chipValues.current) {
        const pools = poolsAt(values, id);
        const text = pools ? formatSigned(balanceOf(pools).net, data.decimals) : 'Not recorded';
        if (element.textContent !== text) element.textContent = text;
      }
    };
    paint();
    return atHead.live.subscribe(paint);
  }, [atHead.data, atHead.live, report]);

  if (atHead.error) return <WidgetState kind="error" failure={atHead.error} onRetry={atHead.retry} />;
  if (!atHead.data) return <WidgetState kind="loading" label="Loading resources…" read={atHead} />;
  if (!lines.length) return <WidgetState kind="empty" icon="layers" message="This recording names no resources." />;
  const error = supply.error ?? demand.error;
  if (error && !supply.summaryByKey.size) return <WidgetState kind="error" failure={error} onRetry={() => { supply.retry(); demand.retry(); }} />;
  const data = atHead.data;
  const tooltip = (at: number) => {
    const reply = hovered.data;
    const values = reply ? repliedValues(reply) : null;
    const shown: LineAt[] = lines.map(line => {
      const pools = values && poolsAt(values, line.resource.id);
      return { resource: line.resource, color: line.color, balance: pools ? balanceOf(pools) : null, deals: reply && values ? dealsOf(reply, line.resource.id, values) : null,
        missing: !reply ? gate.recorded(at, within) === null ? UNRECORDED_MESSAGE : 'Loading…' : !isMember(within, at) ? 'Not in the group at this date' : reply.poolsMissing ?? (values ? poolsMissing(values, line.resource.id) : null) };
    });
    return overTimeTooltip(`${dayLabel(at)} · per day`, shown, data.decimals, labels.labelAt(at), viewport);
  };
  const chips = resources.filter(item => item.ordinal === 1 || chosen.includes(item.token));
  const pick = (token: string) => {
    const action = toggleResource(resources, stored, chosen, token);
    if ('value' in action) setOption?.('resources', action.value);
  };
  const initial = atHead.live.read();
  return <div className="resource-balance" ref={setRoot}>
    <div className="chip-set" role="group" aria-label="Resources">{chips.map(item => {
      const action = toggleResource(resources, stored, chosen, item.token);
      const reason = !setOption ? 'Resource selection unavailable.' : 'reason' in action ? action.reason : undefined;
      const selected = chosen.includes(item.token);
      const line = lines.find(each => each.resource.id === item.id);
      const pools = poolsAt(initial, item.id);
      return <button key={item.id} type="button" className="chip" aria-pressed={selected} aria-disabled={reason ? true : undefined}
        aria-label={resourceName(item)} data-tooltip={reason} onClick={() => { if (!reason) pick(item.token); }}>
        {selected && line && <svg className="resource-series-key" width="28" height="14" aria-hidden="true">
          <line x1="0" y1="3" x2="28" y2="3" stroke={line.color} strokeWidth="2" />
          <line x1="0" y1="11" x2="28" y2="11" stroke={line.color} strokeWidth="2" strokeDasharray={SECONDARY_DASH.join(' ')} />
        </svg>}
        {item.icon && <img src={item.icon} alt="" />}{resourceName(item)}
        {selected && <span className="resource-chip-value" ref={element => { if (element) chipValues.current.set(item.id, element); else chipValues.current.delete(item.id); }}>
          {pools ? formatSigned(balanceOf(pools).net, data.decimals) : 'Not recorded'}</span>}
      </button>;
    })}</div>
    <p className="resource-caption">Per day · solid: supply · dashed: demand{shade ? ' · shaded: surplus and shortfall' : ''}</p>
    {selection && <div className="select-row">Choose a drawn resource on {dayLabel(selection.hour)} <Select autoFocus label="Drawn resource" value="" onChange={id => {
      const resource = chosenInfo.find(each => String(each.id) === id);
      if (resource && selection.ids.includes(id)) { onPick(resource, selection.hour); setSelection(null); }
    }} options={[{ value: '', label: 'Choose resource…', disabled: true },
      ...chosenInfo.filter(resource => selection.ids.includes(String(resource.id))).map(resource => ({ value: String(resource.id), label: resourceName(resource) }))]} /></div>}
    <div className="resource-chart">
      <TimeChart label={name === null ? 'Resources over time' : `Resources of ${name} over time`} extent={extent} playhead={playhead} liveHead={liveHead}
        playback={playback} afterPlayhead={options.afterPlayhead === true} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={motionOf(runtime.clock.getSnapshot())} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: options.log === true }}
        layers={layers} data={chartData} active={active} pickOwner={supply.owner} notRecorded={supply.unrecorded && demand.unrecorded ? supply.unrecorded : null}
        onSummaryView={view => { supply.onSummaryView(view); demand.onSummaryView(view); }}
        onNavigateHour={supply.onNavigateHour} keyboardHour={supply.keyboardHour} keyboardRevision={supply.keyboardRevision}
        onHover={setHoverHour}
        onPick={(hour, context) => {
          const drawn = lines.flatMap(line => [supply.summaryOf(keyId(line.key)), demand.summaryOf(keyId(line.key))]
            .flatMap(summary => summary ? [{ key: String(line.resource.id), summary }] : []));
          const ids = pickSummaryLines(drawn, hour, new Set(lines.map(line => String(line.resource.id))), context);
          if (ids.length === 1) {
            const resource = chosenInfo.find(each => String(each.id) === ids[0]);
            if (resource) { setSelection(null); onPick(resource, Math.floor(hour)); }
          } else setSelection(ids.length ? { hour: Math.floor(hour), ids } : null);
        }}
        revision={`${track.branch.id}:${tag}:${lines.map(line => line.key.part).join(',')}:${shade}:${until}:${hovered.data ? 'hover' : ''}:${labels.revision}`}
        tooltip={at => tooltip(Math.floor(at))}
        onContextMenu={(at, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, at)])} />
    </div>
    {menu.menu}
  </div>;
}

function repliedValues(reply: ResourceBalanceReply): LiveValues<number> {
  return repliedCellsOf(reply).read();
}

const replied = new WeakMap<object, Live<LiveValues<number>>>();
function repliedCellsOf(reply: ResourceBalanceReply): Live<LiveValues<number>> {
  let live = replied.get(reply);
  if (!live) { live = repliedCells(reply); replied.set(reply, live); }
  return live;
}
