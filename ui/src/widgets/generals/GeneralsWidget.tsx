import { contentTitle } from '../../dashboard/contentTitles.ts';
import { useCallback, useEffect, useMemo,  useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED } from '../../charts/drill/drillState';
import { useDrill } from '../../charts/drill/useDrill';
import { jumpItem } from '../../charts/jump';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { useHeadProduct } from '../../dashboard/headProduct';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useCountriesSelectionAt } from '../../dashboard/useCountriesSelectionAt';
import { useShownHour } from '../../dashboard/widgetPerf.ts';
import { resolveOptions } from '../../dashboard/options';
import CareerView from '../general-career/CareerView';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { attributeCells, attributesHeader } from './Attributes.tsx';
import { LeaderIdentity, leaderIdentityProps, leaderPortraitProps, SkillStars } from './LeaderIdentity.tsx';
import { useLeaderPortraits } from './portraits';
import type { PortraitAt } from './portraits';
import { attributeNames, leaderKey as rowKey, leaderRequestSelection, leaderRows, leaderSkill as skillValue, leaderTooltip, leadersStructure, pinnedCareer,
  recordedSince, validateLeaders, withheldLeaderState, withheldLeaders } from './generalsData';
import type { LeaderRow } from './generalsData';
import './generals.css';
import type { LeaderKinds } from '../../generated/dashboard';
import { useRegion, useRegionHour } from '../../dashboard/widgetTime.ts';

const EMPTY: readonly LeaderRow[] = [];
const wonValue = (row: LeaderRow) => row.won;
const lostValue = (row: LeaderRow) => row.lost;
const unresolvedValue = (row: LeaderRow) => row.unresolved;
const winRateValue = (row: LeaderRow) => row.winRate;

const partialCount = (row: LeaderRow, count: number) => row.foughtPartial
  ? <span data-tooltip="Battle counts are incomplete: some battles were not recorded.">{count}</span> : count;

export default function GeneralsWidget(props: WidgetProps) {
  const { widget, runtime, active, onDiagnostics, setOption } = props;
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const owner = useMemo(() => ({}), [runtime.client, track?.source.id, track?.source.revision, track?.branch.id]);
  const currentOwner = useRef(owner);
  currentOwner.current = owner;
  const [drillOwner, setDrillOwner] = useState<object | null>(null);
  const drillControl = useDrill(runtime, useRegion(widget));
  const savedDrill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const drill = drillOwner === owner ? savedDrill : CLOSED;
  const drillRegion = drillControl.region;
  const drillAt = useRegionHour(runtime, drillRegion, active && drill.breakdown != null);
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const options = resolveOptions(OPTIONS, widget.options);
  const kind = String(options.leaders) as LeaderKinds;
  const members = useCountriesSelectionAt(widget, runtime, active);
  const params = useMemo(() => members.requestTags === undefined ? null : { selection: leaderRequestSelection(members.requestTags), leaders: kind },
    [members.requestTags, kind]);
  const head = useHeadProduct(runtime, members.gate, 'unit-leaders', params, { active, validate: validateLeaders, structure: leadersStructure });
  const hour = members.outside ? null : head.at ?? members.hour;
  const landed = hour === null ? { countries: members.countries, reason: null } : members.countriesAt(hour);
  const selection = { group: members.group, countries: landed.countries, selectionReason: members.selectionReason ?? landed.reason };
  const shown = useShownHour();
  shown(head.data ? hour : null);
  const data = head.data;
  const rows = useMemo(() => data && hour !== null && selection.countries !== undefined ? leaderRows(data, hour, selection.countries, kind) : EMPTY, [data, hour, selection.countries, kind]);
  const [visible, setVisible] = useState<readonly LeaderRow[]>(EMPTY);
  const portraitsAt = useMemo<PortraitAt[]>(() => visible.flatMap(row => { const at = row.row.firstSeenHour ?? hour; return at === null ? [] : [{ reference: row.key, hour: at }]; }), [visible, hour]);
  const portraits = useLeaderPortraits(runtime, track, portraitsAt, 86, active && drill.breakdown == null);
  const portraitOf = (row: LeaderRow) => { const at = row.row.firstSeenHour ?? hour; return at === null ? undefined : portraits.at({ reference: row.key, hour: at }); };
  const attributeMaxima = useMemo(() => [0, 1, 2, 3].map(i => rows.reduce((max, row) => Math.max(max, row.row.components[i] ?? 0), 0)), [rows]);
  const wanted = useMemo(() => visible.flatMap(row => row.role.owner ? [{ hour: row.hour, tag: row.role.owner }] : []), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const viewportHeight = useViewportHeight();
  const anchor = useRef<HTMLDivElement>(null);
  const open = useCallback((row: LeaderRow) => {
    if (currentOwner.current !== owner) return;
    setDrillOwner(owner);
    drillControl.show(row.key, row.hour);
  }, [owner, drillControl]);
  useEffect(() => { dispatch({ type: 'close' }); setDrillOwner(null); }, [owner]);
  const diagnostics = useMemo(() => [...new Set([...(data?.warnings ?? []), ...portraits.warnings])], [data, portraits.warnings]);
  useDiagnostics(onDiagnostics, drill.breakdown == null ? diagnostics : null);
  const attributeLabels = useMemo(() => attributeMaxima.map((_, i) =>
    [...new Set(rows.map(row => attributeNames(row.role)[i]))].join(' / ') || 'Name not recorded'), [rows, attributeMaxima]);
  const country = useCallback((row: LeaderRow) => row.role.owner ? labelAt(row.hour)(row.role.owner) : { name: 'Owner not recorded' }, [labelAt]);
  const columns = useMemo<readonly TableColumn<LeaderRow>[]>(() => [
    { key: 'leader', header: 'Leader', label: 'Leader', sortable: false, width: 'minmax(190px, 2fr)', value: row => row.name,
      render: row => <LeaderIdentity {...leaderIdentityProps({ name: row.name, role: row.role, skill: row.row.skill }, country(row))} {...leaderPortraitProps(portraitOf(row))} /> },
    { key: 'skill', header: 'Skill', label: 'Skill', width: '92px', value: skillValue,
      renderMissing: true, render: r => <SkillStars level={r.row.skill} maximum={r.role.maximumSkill} /> },
    { key: 'attributes', headerRevision: JSON.stringify(attributeLabels), header: attributesHeader({ names: attributeLabels }),
      label: 'Attributes', sortable: false, width: '150px', priority: 4, inTooltip: true, value: () => null, renderMissing: true,
      widthKey: row => [...row.row.components, ...attributeNames(row.role)],
      render: row => attributeCells({ values: row.row.components, maxima: attributeMaxima, names: attributeNames(row.role), bars: options.cellBars === true }) },
    { key: 'traits', header: 'Traits', label: 'Traits', sortable: false, width: '1.1fr', priority: 5, inTooltip: true, value: r => r.row.traits?.length,
      renderMissing: true, render: r => <span className="leader-traits">{r.row.traits == null ? <span className="leader-missing">Not recorded</span> : r.traits.map(trait => {
        const name = contentTitle(trait.name, trait.token, 'Trait name unavailable');
        return trait.icon ? <img key={trait.token} src={trait.icon} alt={name} /> : <span className="leader-trait-chip" key={trait.token} data-detail={trait.missing ?? undefined}>{name}</span>;
      })}</span> },
    { key: 'won', header: 'Won', label: 'Battles won', numeric: true, width: '64px', value: wonValue, renderMissing: true,
      render: r => r.won != null ? partialCount(r, r.won) : <span className="leader-missing" data-tooltip="Result not recorded">—</span> },
    { key: 'lost', header: 'Lost', label: 'Battles lost', numeric: true, width: '64px', priority: 1, inTooltip: true, value: lostValue, renderMissing: true,
      render: r => r.lost != null ? partialCount(r, r.lost) : <span className="leader-missing" data-tooltip="Result not recorded">—</span> },
    { key: 'unresolved', header: 'Unresolved', label: 'Unresolved battles', numeric: true, width: '88px', priority: 3, inTooltip: true, value: unresolvedValue, renderMissing: true,
      render: r => r.foughtPartial ? partialCount(r, r.unresolved) : <span data-tooltip="Battles with no known result.">{r.unresolved}</span> },
    { key: 'fought', header: 'Fought', label: 'Fought', sortable: false, numeric: true, width: '64px', priority: 2, inTooltip: true, value: r => r.fought, renderMissing: true,
      render: r => r.foughtPartial ? partialCount(r, r.fought) : <span data-tooltip={recordedSince(r.row.firstSeenHour, track?.branch.firstHour ?? 0)}>{r.fought}</span> },
    { key: 'winRate', header: 'Win %', label: 'Win rate among decided battles', numeric: true, width: '56px', value: winRateValue, renderMissing: true,
      render: r => r.winRate == null ? <span className="leader-missing" data-tooltip="No battles with known results.">—</span>
        : <span data-tooltip={r.foughtPartial ? 'Win rate among the decided battles recorded; some battles were not recorded.' : 'Win rate among decided battles'}>{Math.round(r.winRate)}%</span> },
  ], [country, portraits, attributeMaxima, attributeLabels, options.cellBars, track?.branch.firstHour]);
  const tooltip = useCallback((row: LeaderRow) => fitTooltip([leaderTooltip(row, country(row))], viewportHeight), [country, viewportHeight]);
  const body = () => {
    if (!track) return <WidgetState kind="empty" message="Load a campaign to see its leaders." />;
    if (head.outside || members.outside) return <WidgetState kind="outside" read={head} />;
    if (head.error) return <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
    if (selection.group.error) return <WidgetState kind="error" failure={selection.group.error} onRetry={selection.group.retry} />;
    if (selection.selectionReason) return <WidgetState kind="unavailable" message={selection.selectionReason} />;
    if (!data) return <WidgetState kind="loading" label="Loading leaders…" />;
    if (selection.countries === undefined) return <WidgetState kind="unavailable" message="Country group membership was not recorded at this hour." />;
    if (data.missing) return <WidgetState kind="unavailable" message="Leaders weren’t recorded at this time." detail={data.missing} />;
    const withheld = withheldLeaders(data, selection.countries, kind);
    const withheldState = withheldLeaderState(withheld);
    if (!rows.length && withheld.length) return <WidgetState kind="unavailable" {...withheldState} />;
    if (!rows.length) return <WidgetState kind="empty" message="No leaders for these countries at this time." />;
    return <div className="leader-table">
      <Table rowRevision={row => [row, columns]} key={JSON.stringify([track.source.id, track.source.revision, track.branch.id])} label="Generals leaderboard" rows={rows} columns={columns} rowKey={rowKey} rowHeight={48} rank
        sort={tableSortOf(options.tableSort, DEFAULT_SORT)} setOption={setOption} cellBars={options.cellBars === true} rankMarkers={options.rankMarkers === true}
        density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} onRowClick={open} tooltip={tooltip} onVisibleRows={setVisible}
        menu={r => { const first = r.row.firstSeenHour;
          return first === null ? [{ label: 'Open career', run: () => open(r) }]
            : [{ ...jumpItem(runtime, track.id, first), label: `Jump to first seen ${formatDateTime(first)}` }, { label: 'Open career', run: () => open(r) }]; }}
        hoursPerSecond={hoursPerSecond} />
      {withheld.length > 0 && <details className="career-note"><summary>Some leaders are missing.</summary><p className="leader-withheld-detail">{withheldState.detail}</p></details>}
    </div>;
  };
  return <div ref={anchor} className="generals-widget"><DrillHost drill={drillControl} runtime={runtime} subject={drill.breakdown != null} mode={options.drill === 'popout' ? 'popout' : 'inline'}
    title="General career" day={drillAt === null ? '' : formatDateTime(drillAt)} canFollow anchor={anchor}
    onClose={() => { if (currentOwner.current === owner) dispatch({ type: 'close' }); }}
    pinRefused={drillControl.pinRefused} onPin={drill.breakdown && track && drillControl.time() ? () => {
      const time = drillControl.time();
      const history = time?.value.mode === 'pin' ? drillControl.gate.span(track.branch.firstHour, drillControl.hour ?? Number.NaN) : null;
      const kept = history ? drillControl.time(history) : time;
      if (currentOwner.current === owner && kept) runtime.addWidgetBelow(widget.id, pinnedCareer(widget, drill.breakdown!, track, kept));
    } : undefined} main={<div ref={head.root} className="generals-main">{body()}</div>}>
    {drill.breakdown && <CareerView {...props} time={drillRegion.action}
      reference={drill.breakdown} onPick={h => {
        if (currentOwner.current === owner) dispatch({ type: 'pick', hour: h });
      }} />}
  </DrillHost></div>;
}
