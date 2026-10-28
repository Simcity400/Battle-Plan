import { useCallback, useEffect, useId, useMemo, useReducer, useRef, useState } from 'react';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/columns';
import { nameKey, useCountryNamesRead } from '../../dashboard/countryNamesAt';
import { nameReadText, useHolderLooks } from '../../dashboard/holderLooks';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import WidgetState from '../../components/WidgetState';
import { Segmented } from '../../components/Segmented';
import { dayLabel } from '../../charts/draw';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, reduceDrill } from '../../charts/drill/drillState';
import ManpowerHistory from './ManpowerHistory';
import { usNumber } from '../../charts/numberFormat';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { useRowTooltips } from '../../charts/useRowTooltips';
import { formatAmount } from '../../charts/valueAxis';
import { useTableHour } from '../../charts/table/useTableHour';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { factionNameAt, noFactionMessage, useChosenFaction, useFactionDetails } from '../../dashboard/factionChoice';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { OPTIONS } from './definition';
import { POOL_CHOICES, manpowerRows, poolHeading, poolsOffered, shareOf, shownPool } from './poolsData';
import type { MemberAmount } from './poolsData';
import '../resource-balance/resourceBalance.css';
import './factionPooledStrength.css';

const NO_WARNINGS: string[] = [];
const memberKey = (entry: MemberAmount) => entry.key;
/** Recorded pool amounts are fixed-point words with five decimals. */
const SCALE = 5;
const bar = (value: bigint | number, largest: bigint | number) => {
  const [v, l] = [Number(value), Number(largest)];
  return l > 0 && v > 0 ? `${Math.min(100, v / l * 100)}%` : '0%';
};

export default function FactionPooledStrengthWidget({ widget, runtime, active, headerSlot, setOption, onDiagnostics }: WidgetProps) {
  const playback = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const tokens = useChartTokens();
  const owner = useId();
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playback);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const { groups, pick } = useChosenFaction(widget, runtime, track, hour, active);
  const faction = pick?.faction ?? null;
  const details = useFactionDetails(runtime, track, faction?.id ?? null, hour, active);
  const data = details.data;
  // Details still shown while the next hour loads keep the hour they answer.
  const dataHour = data && details.moment ? (JSON.parse(details.moment) as [number])[0] : hour;
  const offered = useMemo(() => data ? poolsOffered(data) : [], [data]);
  const pool = shownPool(offered, options.pool);
  const members = useMemo(() => data ? manpowerRows(data) : [], [data]);
  // Names for the selected pool's whole list; flags only for the rows a windowed list shows.
  const tags = useMemo(() => [...new Set(pool === 'manpower' ? members.flatMap(row => row.tag ? [row.tag] : [])
    : pool === 'research' ? data?.researchCountries ?? [] : [])], [pool, members, data]);
  const [savedDrill, dispatch] = useReducer(reduceDrill, CLOSED);
  // A drill belongs to the faction it was opened from; another source, branch or faction closes it.
  const drillOwner = JSON.stringify([track?.source.id ?? null, track?.source.revision ?? null, track?.branch.id ?? null, faction?.id ?? null]);
  const [openedFor, setOpenedFor] = useState<string | null>(null);
  useEffect(() => { dispatch({ type: 'close' }); setOpenedFor(null); }, [drillOwner]);
  const drill = openedFor === drillOwner ? savedDrill : CLOSED;
  // The drill keeps the member clicked as it was at the hour clicked (`drill.hour`).
  const picked = drill.breakdown;
  const pickedHour = drill.hour ?? dataHour;
  const pickedHolder = useMemo(() => picked ? [{ key: 'picked', tag: picked, hour: pickedHour }] : [], [picked, pickedHour]);
  const pickedLook = useHolderLooks(runtime.client, track, pickedHolder, active, tokens);
  const names = useCountryNamesRead(runtime.client, track, useMemo(() => tags.map(tag => ({ hour: dataHour, tag })), [tags, dataHour]), active);
  // Flags only for the manpower rows the mounted list shows; leaving the list releases them.
  const listed = pool === 'manpower' && members.length > 0;
  const [visibleTags, setVisibleTags] = useState<readonly string[]>([]);
  useEffect(() => { if (!listed) setVisibleTags(previous => previous.length ? [] : previous); }, [listed]);
  const flags = useCountryLabelsAt(runtime.client, track, useMemo(() => listed ? visibleTags.map(tag => ({ hour: dataHour, tag })) : [], [listed, visibleTags, dataHour]), active);
  const label = useCallback((tag: string) => ({ ...flags.labelAt(dataHour)(tag), name: nameReadText(names.get(nameKey(dataHour, tag))) }), [flags, names, dataHour]);
  const diagnostics = useMemo(() => !data ? (details.error ? [details.error] : NO_WARNINGS)
    : [...data.warnings, ...data.refusals.map(refusal => `${refusal.fact} (${refusal.count}): ${refusal.reason}`),
      ...((data.resources ?? []).filter(entry => entry.reason).length ? [`${(data.resources ?? []).filter(entry => entry.reason).length} pooled resource names are unavailable.`] : [])],
  [data, details.error]);
  useDiagnostics(onDiagnostics, diagnostics);
  const content = useCallback((key: string): TooltipContent | null => {
    if (!data || !pool) return null;
    const subtitle = `${faction ? factionNameAt(faction, dataHour) : ''} · ${dayLabel(dataHour)}`;
    if (pool === 'resources') {
      const entry = data.resources?.find(item => String(item.ordinal) === key);
      return entry ? { title: entry.name ?? 'Resource name unavailable', subtitle, rows: [{ label: 'Pooled', value: formatAmount(entry.raw, SCALE, 5) }] } : null;
    }
    if (pool === 'manpower') {
      const entry = members.find(item => item.key === key);
      if (!entry) return null;
      const country = entry.tag ? label(entry.tag) : { name: 'Country unavailable' };
      return { title: country.name, countryFlag: entry.tag ? country : undefined, subtitle, rows: [
        { label: 'Manpower contributed', value: usNumber.format(entry.value) },
        { label: 'Share of the pool', value: shareOf(entry.value, data.manpowerTotal) },
      ] };
    }
    if (pool === 'research') {
      if (key === 'base') return { title: 'Scientist base', subtitle, rows: [{ label: 'Value', value: data.researchBaseRaw == null ? 'Not recorded' : formatAmount(data.researchBaseRaw, SCALE, 5) }] };
      if (data.researchCountries == null) return { title: 'Countries sharing research', subtitle, rows: [{ label: 'Countries', value: 'Not recorded' }] };
      const country = label(key);
      return { title: country.name, countryFlag: country, subtitle, rows: [{ label: 'Research', value: 'Shares research with the faction' }] };
    }
    const folder = data.doctrineFolders?.find(item => String(item.ordinal) === key);
    return folder ? { title: folder.name ?? 'Doctrine name unavailable', subtitle, rows: [{ label: 'Doctrine', value: 'Shared with the faction' }] } : null;
  }, [data, pool, faction, dataHour, members, label]);
  const tips = useRowTooltips<string>({ owner, active, content, revision: `${dataHour}:${pool}`, onMenu: () => {} });
  const anchor = useRef<HTMLDivElement>(null);
  const open = (tag: string | null) => { if (tag) { setOpenedFor(drillOwner); dispatch({ type: 'open', breakdown: tag, hour: dataHour, playing: false, playhead: null }); } };
  const manpowerColumns = useMemo<readonly TableColumn<MemberAmount>[]>(() => [
    { key: 'member', header: 'Member', label: 'Member', width: 'minmax(var(--table-name-min), 2fr)', value: entry => { const name = entry.tag ? names.get(nameKey(dataHour, entry.tag)) : null; return typeof name === 'string' ? name : null; },
      renderMissing: true, render: entry => entry.tag ? <CountryCell {...label(entry.tag)} /> : 'Country unavailable' },
    { key: 'manpower', header: 'Manpower', label: 'Manpower contributed', numeric: true, width: 'max-content', bar: 'visible', value: entry => entry.value,
      render: entry => usNumber.format(entry.value) },
    { key: 'share', header: 'Share', label: 'Share of the pool', numeric: true, width: 'max-content', priority: 1,
      value: entry => data?.manpowerTotal ? entry.value / data.manpowerTotal : null, render: entry => shareOf(entry.value, data?.manpowerTotal ?? null) },
  ], [names, label, dataHour, data?.manpowerTotal]);
  const memberTip = useCallback((entry: MemberAmount) => content(entry.key)!, [content]);
  const header = headerSlot && data && pool && createPortal(<span className="pill">{poolHeading(data, pool)}</span>, headerSlot);
  if (!track) return <WidgetState kind="empty" icon="layers" message="Load a campaign to see faction pools." />;
  if (!window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  const groupFailure = groups.error ? <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (!groups.data) return <>{groupFailure}<WidgetState kind="loading" label="Loading faction pools…" /></>;
  if (!faction) return <>{groupFailure}<WidgetState kind="empty" icon="layers" message={noFactionMessage(pick!.reason as 'none' | 'unobserved' | 'unknown', widget.time.mode === 'follow')} /></>;
  if (details.error) return <>{groupFailure}<WidgetState kind="error" detail={details.error} onRetry={details.retry} /></>;
  if (!data) return <>{groupFailure}<WidgetState kind="loading" label="Loading faction pools…" /></>;
  if (!pool) return <>{groupFailure}<WidgetState kind="unavailable" message="This faction’s pools were not recorded at this time." /></>;
  const row = (key: string, name: ReactNode, track: ReactNode, value: ReactNode, pick?: () => void) => <li key={key} {...tips.rowProps(key)}
    {...(pick ? { role: 'button', onClick: pick, onKeyDown: (event: React.KeyboardEvent<HTMLLIElement>) => {
      if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); pick(); } else tips.rowProps(key).onKeyDown(event); } } : {})}>
    {name}{track}{value}</li>;

  let body: ReactNode;
  if (pool === 'resources') {
    const entries = data.resources ?? [];
    const largest = entries.reduce((max, entry) => BigInt(entry.raw) > max ? BigInt(entry.raw) : max, 0n);
    body = !entries.length ? <WidgetState kind="empty" icon="layers" message="This faction pools no resources yet." />
      : <ol className="resource-bars" aria-label={`Pooled resources on ${dayLabel(dataHour)}`}>{entries.map(entry => row(String(entry.ordinal),
        <span className="resource-name">{entry.icon ? <img src={entry.icon} alt="" /> : null}<span title={entry.name ?? 'Resource name unavailable'}>{entry.name ?? 'Resource name unavailable'}</span></span>,
        <span className="resource-track" aria-hidden="true"><span className="resource-bar"><i style={{ width: bar(BigInt(entry.raw), largest), background: tokens.categories[0] }} /></span></span>,
        <b>{formatAmount(entry.raw, SCALE)}</b>))}</ol>;
  } else if (pool === 'manpower') {
    body = !members.length ? <WidgetState kind="empty" icon="layers" message="No member contributes manpower yet." />
      : <Table label={`Manpower contributions on ${dayLabel(dataHour)}`} rows={members} columns={manpowerColumns} rowKey={memberKey}
        sort={{ column: 'manpower', direction: 'descending' }} rankMarkers={false} tooltip={memberTip} onRowClick={entry => open(entry.tag)}
        onVisibleRows={rows => setVisibleTags(previous => { const next = [...new Set(rows.flatMap(entry => entry.tag ? [entry.tag] : []))];
          return previous.length === next.length && previous.every((tag, index) => tag === next[index]) ? previous : next; })} />;
  } else if (pool === 'research') {
    body = <ol className="resource-bars" aria-label={`Research sharing on ${dayLabel(dataHour)}`}>
      {row('base', <span className="resource-name"><span>Scientist base</span></span>, <span />, <b>{data.researchBaseRaw == null ? 'Not recorded' : formatAmount(data.researchBaseRaw, SCALE)}</b>)}
      {data.researchCountries == null ? row('members', <span className="resource-name"><span>Countries sharing research</span></span>, <span />, <b>Not recorded</b>)
        : data.researchCountries.map(tag => row(tag, <span className="resource-name"><span title={label(tag).name}>{label(tag).name}</span></span>, <span />, <span />))}
    </ol>;
  } else {
    body = <ol className="resource-bars" aria-label={`Shared doctrines on ${dayLabel(dataHour)}`}>{(data.doctrineFolders ?? []).map(folder => row(String(folder.ordinal),
      <span className="resource-name"><span title={folder.name ?? 'Doctrine name unavailable'}>{folder.name ?? 'Doctrine name unavailable'}</span></span>, <span />, <span />))}</ol>;
  }
  return <div ref={anchor} className="faction-pooled">{groupFailure}
    {header}
    {setOption && offered.length > 1 && <div className="faction-pooled-caption">
      <Segmented value={pool} options={POOL_CHOICES.filter(choice => offered.includes(choice.value))} label="Pool" onChange={value => setOption('pool', value)} />
    </div>}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={picked != null} title="Manpower over time" day={dayLabel(pickedHour)}
      following={false} canFollow={false} anchor={anchor} onClose={() => dispatch({ type: 'close' })} onFollow={() => {}} main={body}>
      {picked && track && <ManpowerHistory runtime={runtime} track={track} faction={faction.id} tag={picked} picked={pickedHour} name={pickedLook.name('picked')}
        colour={pickedLook.colour('picked')} hour={hour} active={active} />}
    </DrillHost>
  </div>;
}
