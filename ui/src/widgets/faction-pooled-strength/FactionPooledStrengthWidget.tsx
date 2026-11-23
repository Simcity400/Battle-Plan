import { useCallback, useEffect, useId, useLayoutEffect, useMemo,  useRef } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/columns';
import { nameKey, useCountryNamesAt } from '../../dashboard/countryNamesAt.ts';
import { useHolderLooks } from '../../dashboard/holderLooks.ts';
import { nameReadText, nameReadMetadata } from '../../dashboard/nameReads.ts';
import WidgetState from '../../components/WidgetState';
import { useWidgetViewSwitch } from '../../dashboard/widgetFrame';
import { dayLabel } from '../../charts/draw';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import LiveText from '../../charts/LiveText';
import type { LiveTexts } from '../../charts/LiveText';
import { mapLive } from '../../charts/live';
import { usNumber } from '../../charts/numberFormat';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { factionNameAt, noFactionMessage, useChosenFaction, useFactionDisplayNames, useFactionDetails } from '../../dashboard/factionChoice';
import type { NoFaction } from '../../dashboard/factionChoice';
import { liveTextsOf, manpowerTotalKey, researchBaseKey, resourceKey } from '../../dashboard/factionDetails.ts';
import { reporting } from '../../dashboard/follow/liveCells.ts';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { OPTIONS } from './definition';
import ManpowerHistory from './ManpowerHistory';
import { POOL_CHOICES, manpowerRows, manpowerTableValues, memberAmount, poolHeading, poolsOffered, shareOf, shownPool } from './poolsData';
import type { MemberAmount } from './poolsData';
import '../resource-balance/resourceBalance.css';
import './factionPooledStrength.css';
import { useChangedState } from '../../dashboard/changedState.ts';
import { useRegion } from '../../dashboard/widgetTime.ts';

const NO_WARNINGS: string[] = [];
const NO_TAGS: readonly string[] = [];
const memberKey = (entry: MemberAmount) => entry.key;
const amount = new Intl.NumberFormat('en-US', { maximumFractionDigits: 2 });
const exact = new Intl.NumberFormat('en-US', { maximumFractionDigits: 5 });

function LiveWidth({ live, at, colour }: { live: LiveTexts; at: string; colour: string }) {
  const element = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    const paint = () => { const node = element.current, width = live.text(at); if (node && node.style.width !== width) node.style.width = width; };
    paint();
    return live.subscribe(paint);
  }, [live, at]);
  return <i ref={element} style={{ width: live.text(at), background: colour }} />;
}

export default function FactionPooledStrengthWidget({ widget, runtime, active, headerSlot, setOption, onDiagnostics }: WidgetProps) {
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const owner = useId();
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const { groups, pick, hour } = useChosenFaction(widget, runtime, track, active);
  const displayNames = useFactionDisplayNames(track?.source);
  const faction = pick?.faction ?? null;
  const head = useFactionDetails(runtime, widget, faction?.id ?? null, active);
  const data = head.data && head.data.faction === faction?.id ? head.data : undefined;
  const dataHour = head.at ?? hour ?? track?.branch.lastHour ?? 0;
  const shown = useShownHour();
  const shownAt = useRef(head.shownAt);
  shownAt.current = head.shownAt;
  const live = useMemo(() => reporting(head.live, () => shown(shownAt.current())), [head.live, shown]);
  const offered = useMemo(() => data ? poolsOffered(data) : [], [data]);
  const pool = shownPool(offered, options.pool);
  const members = useMemo(() => data ? manpowerRows(data) : [], [data]);
  const resources = data?.resources ?? null;
  const texts = useMemo(() => liveTextsOf(live, (values, key) => {
    if (key === 'total') return data && pool ? poolHeading(data, pool, values) : '';
    if (key === 'research') { const value = values.get(researchBaseKey); return value === undefined ? values.missing(researchBaseKey) || 'Not recorded' : amount.format(value); }
    const [kind, ordinal] = key.split(':');
    const value = values.get(resourceKey(Number(ordinal)));
    if (kind === 'amount') return value === undefined ? values.missing(resourceKey(Number(ordinal))) || 'Not recorded' : amount.format(value);
    const largest = Math.max(0, ...(resources ?? []).map(entry => values.get(resourceKey(entry.ordinal)) ?? 0));
    return value !== undefined && value > 0 && largest > 0 ? `${Math.min(100, value / largest * 100)}%` : '0%';
  }), [live, data, pool, resources]);
  const tableLive = useMemo(() => mapLive(live, manpowerTableValues), [live]);
  const tags = useMemo(() => [...new Set(pool === 'manpower' ? members.flatMap(row => row.tag ? [row.tag] : [])
    : pool === 'research' ? data?.researchCountries ?? [] : [])], [pool, members, data]);
  const drillControl = useDrill(runtime, useRegion(widget));
  const savedDrill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const drillOwner = JSON.stringify([track?.source.id ?? null, track?.source.revision ?? null, track?.branch.id ?? null]);
  useEffect(() => { dispatch({ type: 'close' }); }, [drillOwner]);
  const drill = savedDrill;
  const picked = useMemo(() => drill.breakdown === null ? null : JSON.parse(drill.breakdown) as { faction: string; tag: string }, [drill.breakdown]);
  const pickedHour = drillControl.hour;
  const pickedHolder = useMemo(() => picked && pickedHour !== null ? [{ key: 'picked', tag: picked.tag, hour: pickedHour }] : [], [picked, pickedHour]);
  const pickedLook = useHolderLooks(runtime.client, track, pickedHolder, active, tokens);
  const names = useCountryNamesAt(runtime.client, track, useMemo(() => tags.map(tag => ({ hour: dataHour, tag })), [tags, dataHour]), active);
  const listed = pool === 'manpower' && members.length > 0;
  const [visibleTags, setVisibleTags] = useChangedState<readonly string[]>(NO_TAGS);
  useEffect(() => { if (!listed) setVisibleTags(previous => previous.length ? NO_TAGS : previous); }, [listed]);
  const researchTags = pool === 'research' ? data?.researchCountries ?? null : null;
  const flags = useCountryLabelsAt(runtime.client, track, useMemo(() => (listed ? visibleTags : researchTags ?? []).map(tag => ({ hour: dataHour, tag })),
    [listed, visibleTags, researchTags, dataHour]), active);
  const label = useCallback((tag: string) => ({ ...flags.labelAt(dataHour)(tag), ...(names.has(nameKey(dataHour, tag)) ? nameReadMetadata(names.get(nameKey(dataHour, tag))) : {}) }), [flags, names, dataHour]);
  const diagnostics = useMemo(() => !data ? (head.error ? [head.error.message] : NO_WARNINGS)
    : [...data.warnings, ...data.refusals.map(refusal => `${refusal.fact} (${refusal.count}): ${refusal.reason}`),
      ...((data.resources ?? []).filter(entry => entry.reason).length ? [`${(data.resources ?? []).filter(entry => entry.reason).length} pooled resource names are unavailable.`] : [])],
  [data, head.error]);
  useDiagnostics(onDiagnostics, diagnostics);
  const content = useCallback((key: string): TooltipContent | null => {
    if (!data || !pool) return null;
    const values = live.read();
    const subtitle = `${faction ? factionNameAt(faction, dataHour, displayNames[faction.id]) : ''} · ${dayLabel(dataHour)}`;
    if (pool === 'resources') {
      const entry = data.resources?.find(item => String(item.ordinal) === key);
      if (!entry) return null;
      const value = values.get(resourceKey(entry.ordinal));
      return { title: entry.name ?? 'Resource name unavailable', subtitle, rows: [{ label: 'Pooled', value: value === undefined ? 'Not recorded' : exact.format(value) }] };
    }
    if (pool === 'manpower') {
      const entry = members.find(item => item.key === key);
      if (!entry) return null;
      const country = entry.tag ? label(entry.tag) : { name: 'Country unavailable' };
      const value = memberAmount(values, entry.ordinal);
      return { title: country.name, countryFlag: entry.tag ? country : undefined, subtitle, rows: [
        { label: 'Manpower contributed', value: value === undefined ? 'Not recorded' : usNumber.format(value) },
        { label: 'Share of the pool', value: shareOf(value, values.get(manpowerTotalKey)) },
      ] };
    }
    if (pool === 'research') {
      if (key === 'base') { const value = values.get(researchBaseKey); return { title: 'Scientist base', subtitle, rows: [{ label: 'Value', value: value === undefined ? 'Not recorded' : exact.format(value) }] }; }
      if (data.researchCountries === null) return { title: 'Countries sharing research', subtitle, rows: [{ label: 'Countries', value: 'Not recorded' }] };
      const country = label(key);
      return { title: country.name, countryFlag: country, subtitle, rows: [{ label: 'Research', value: 'Shares research with the faction' }] };
    }
    const folder = data.doctrineFolders?.find(item => String(item.ordinal) === key);
    return folder ? { title: folder.name ?? 'Doctrine name unavailable', subtitle, rows: [{ label: 'Doctrine', value: 'Shared with the faction' }] } : null;
  }, [data, pool, faction, dataHour, members, label, live, displayNames]);
  const tips = useRowTooltips<string>({ owner, active, content, revision: `${dataHour}:${pool}`, onMenu: () => {} });
  const anchor = useRef<HTMLDivElement>(null);
  const open = (tag: string | null) => { if (tag && faction) drillControl.showAt(JSON.stringify({ faction: faction.id, tag }), dataHour); };
  const manpowerColumns = useMemo<readonly TableColumn<MemberAmount>[]>(() => [
    { key: 'member', header: 'Member', label: 'Member', width: 'minmax(var(--table-name-min), 2fr)', value: entry => { const name = entry.tag ? names.get(nameKey(dataHour, entry.tag)) : undefined; return nameReadText(name); },
      text: entry => entry.tag ? label(entry.tag).name : null, renderMissing: true, render: entry => entry.tag ? <CountryCell {...label(entry.tag)} /> : 'Country unavailable' },
    { key: 'manpower', header: 'Manpower', label: 'Manpower contributed', numeric: true, width: 'max-content', bar: 'visible', value: () => null,
      paint: value => usNumber.format(value as number) },
    { key: 'share', header: 'Share', label: 'Share of the pool', numeric: true, width: 'max-content', priority: 1, value: () => null,
      paint: value => `${(Math.round((value as number) * 1000) / 10).toFixed(1)}%` },
  ], [names, label, dataHour]);
  const memberTip = useCallback((entry: MemberAmount) => content(entry.key)!, [content]);
  const poolSwitch = useMemo(() => setOption && pool && offered.length > 1 ? {
    label: 'Pool',
    value: pool,
    options: POOL_CHOICES.filter(choice => offered.includes(choice.value)),
    onChange: (value: string) => setOption('pool', value),
  } : null, [setOption, pool, offered]);
  useWidgetViewSwitch(poolSwitch);
  const header = headerSlot && data && pool && createPortal(<span className="pill"><LiveText live={texts} at="total" /></span>, headerSlot);
  const following = widget.time.mode === 'follow';
  const body = (() => {
    if (!track) return <WidgetState kind="empty" icon="layers" message="Load a campaign to see faction pools." />;
    if (hour === null) return <WidgetState kind="outside" />;
    const groupFailure = groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
    if (groupFailure && !groups.data) return groupFailure;
    if (!groups.data) return <WidgetState kind="loading" label="Loading faction pools…" />;
    if (!faction) return <>{groupFailure}<WidgetState kind="empty" icon="layers" message={noFactionMessage(pick!.reason as NoFaction, following)} /></>;
    if (head.error && !data) return <>{groupFailure}<WidgetState kind="error" failure={head.error} onRetry={head.retry} /></>;
    if (!data) return <>{groupFailure}<WidgetState kind="loading" label="Loading faction pools…" /></>;
    if (!pool) return <>{groupFailure}<WidgetState kind="unavailable" message="This faction’s pools were not recorded at this time." /></>;
    const row = (key: string, name: ReactNode, track: ReactNode, value: ReactNode, pick?: () => void) => <li key={key} {...tips.rowProps(key)}
      {...(pick ? { role: 'button', onClick: pick, onKeyDown: (event: React.KeyboardEvent<HTMLLIElement>) => {
        if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); pick(); } else tips.rowProps(key).onKeyDown(event); } } : {})}>
      {name}{track}{value}</li>;
    let main: ReactNode;
    if (pool === 'resources') {
      const entries = data.resources ?? [];
      main = !entries.length ? <WidgetState kind="empty" icon="layers" message="This faction pools no resources yet." />
        : <ol className="resource-bars" aria-label={`Pooled resources on ${dayLabel(dataHour)}`}>{entries.map(entry => row(String(entry.ordinal),
          <span className="resource-name">{entry.icon ? <img src={entry.icon} alt="" /> : null}<span title={entry.name ?? 'Resource name unavailable'}>{entry.name ?? 'Resource name unavailable'}</span></span>,
          <span className="resource-track" aria-hidden="true"><span className="resource-bar"><LiveWidth live={texts} at={`width:${entry.ordinal}`} colour={tokens.categories[0]} /></span></span>,
          <b><LiveText live={texts} at={`amount:${entry.ordinal}`} /></b>))}</ol>;
    } else if (pool === 'manpower') {
      main = !members.length ? <WidgetState kind="empty" icon="layers" message="No member contributes manpower yet." />
        : <Table rowRevision={row => [row, manpowerColumns]} label={`Manpower contributions on ${dayLabel(dataHour)}`} rows={members} columns={manpowerColumns} rowKey={memberKey}
          sort={{ column: 'manpower', direction: 'descending' }} rankMarkers={false} tooltip={memberTip} onRowClick={entry => open(entry.tag)}
          hoursPerSecond={hoursPerSecond} live={tableLive}
          onVisibleRows={rows => setVisibleTags(previous => { const next = [...new Set(rows.flatMap(entry => entry.tag ? [entry.tag] : []))];
            return previous.length === next.length && previous.every((tag, index) => tag === next[index]) ? previous : next; })} />;
    } else if (pool === 'research') {
      main = <ol className="resource-bars" aria-label={`Research sharing on ${dayLabel(dataHour)}`}>
        {row('base', <span className="resource-name"><span>Scientist base</span></span>, <span />, <b><LiveText live={texts} at="research" /></b>)}
        {(data.researchCountries ?? []).map(tag => row(tag, <span className="resource-name"><span title={label(tag).name}>{label(tag).name}</span></span>, <span />, <span />))}
      </ol>;
    } else {
      main = <ol className="resource-bars" aria-label={`Shared doctrines on ${dayLabel(dataHour)}`}>{(data.doctrineFolders ?? []).map(folder => row(String(folder.ordinal),
        <span className="resource-name"><span title={folder.name ?? 'Doctrine name unavailable'}>{folder.name ?? 'Doctrine name unavailable'}</span></span>, <span />, <span />))}</ol>;
    }
    return <>{groupFailure}{header}{main}</>;
  })();
  return <div ref={element => { anchor.current = element; head.root(element); }} className="faction-pooled">
    <DrillHost drill={drillControl} runtime={runtime} mode={options.drill === 'popout' ? 'popout' : 'inline'} title="Manpower over time"
      day={pickedHour === null ? '' : dayLabel(pickedHour)} canFollow={false} anchor={anchor} main={body}>
      {picked && pickedHour !== null && track && <ManpowerHistory runtime={runtime} track={track} faction={picked.faction} tag={picked.tag} picked={pickedHour} name={pickedLook.name('picked')}
        colour={pickedLook.colour('picked')} following={false} active={active} />}
    </DrillHost></div>;
}
