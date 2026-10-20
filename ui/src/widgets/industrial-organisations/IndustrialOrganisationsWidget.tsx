import { createPortal } from 'react-dom';
import { useCallback, useMemo, useReducer, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import ArtFallback from '../../components/ArtFallback';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import { dayLabel } from '../../charts/draw';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipRow } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useCountryNamesAt, nameKey } from '../../dashboard/countryNamesAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { preparedAt } from '../../dashboard/economyHistory';
import { gameWindow } from '../../dashboard/gameWindows';
import type { MioPolicySelection } from '../../dashboard/gameWindows';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { ProductionCatalogueReply } from '../../generated/dashboard';
import { PRODUCTION_CATALOGUE } from '../production-licences/licencesData';
import { DEFAULT_SORT, OPTIONS, POLICY } from './definition';
import { fundsText, organisationsAt, policyText, rosterKnown, validateOrganisations, withWindowDiagnostics } from './organisationsData';
import type { OrganisationRow } from './organisationsData';
import '../equipment-stockpile/equipmentStockpile.css';
import './industrialOrganisations.css';

const NO_ROWS: readonly OrganisationRow[] = [];
const NO_WARNINGS: readonly string[] = [];
const NAME_UNAVAILABLE = 'Organisation name unavailable';

/** A country's industrial organisations at the moment shown, largest first: size,
 * funds, traits and policy. A row opens the game's policy screen for it once the
 * game-window renderer provides one. */
export default function IndustrialOrganisationsWidget({ widget, runtime, active, headerSlot, onDiagnostics, setOption }: WidgetProps) {
  const playbackHour = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const tag = selectedTags(widget.countries, track)[0] ?? null;
  const anchor = useRef<HTMLDivElement>(null);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const [picked, setPicked] = useState<MioPolicySelection | null>(null);
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: PRODUCTION_CATALOGUE,
    params: { branchId: track.branch.id, view: 'organisations', tags: [tag] } } : null;
  const { data, error, retry } = useProduct<ProductionCatalogueReply>(runtime.client, request, active, validateOrganisations, track?.source);
  // The open policy window reports its own diagnostics; both reach the card.
  const [windowLines, setWindowLines] = useState<readonly string[]>(NO_WARNINGS);
  const onWindowDiagnostics = useCallback((lines: string[]) =>
    setWindowLines(previous => previous.length === lines.length && previous.every((line, index) => line === lines[index]) ? previous : lines), []);
  const diagnostics = useMemo(() => withWindowDiagnostics(data?.warnings ?? NO_WARNINGS, windowLines), [data?.warnings, windowLines]);
  useDiagnostics(onDiagnostics, diagnostics);
  const rows = useMemo(() => data && tag ? organisationsAt(data, tag, hour) : NO_ROWS, [data, tag, hour]);
  const names = useCountryNamesAt(runtime.client, track, useMemo(() => tag ? [{ hour, tag }] : [], [tag, hour]), active);
  const Chooser = gameWindow('mioPolicyChooser');
  const columns = useMemo<readonly TableColumn<OrganisationRow>[]>(() => [
    { key: 'organisation', header: 'Organisation', label: 'Organisation', width: 'minmax(var(--table-name-min), 2fr)',
      value: row => row.name ?? NAME_UNAVAILABLE,
      render: row => <span className="stockpile-equipment" data-tooltip={row.token || undefined}>
        {row.icon ? <img className="mio-icon" src={row.icon} alt="" /> : <span className="mio-icon"><ArtFallback icon="factory" /></span>}
        <span><b>{row.name ?? NAME_UNAVAILABLE}</b></span></span> },
    { key: 'size', header: 'Size', label: 'Size', numeric: true, width: 'max-content', renderMissing: true,
      value: row => row.size, render: row => row.size === null ? '—' : row.size.toString() },
    { key: 'funds', header: 'Funds', label: 'Funds', numeric: true, width: 'max-content', renderMissing: true,
      value: row => row.funds, text: row => fundsText(row.funds), render: row => fundsText(row.funds) },
    { key: 'traits', header: 'Traits', label: 'Traits unlocked', numeric: true, width: 'max-content', priority: 2, renderMissing: true,
      value: row => row.traits, render: row => row.traits === null ? '—' : row.traits.toString() },
    { key: 'policy', header: 'Policy', label: 'Policy', width: 'minmax(120px, 1.4fr)', priority: 1, renderMissing: true,
      value: row => row.policy, text: policyText,
      render: row => <span className={row.policy === null ? 'mio-muted' : undefined} data-tooltip={row.policyToken ?? undefined}>{policyText(row)}</span> },
  ], []);
  const tooltip = useCallback((row: OrganisationRow) => {
    const lines: TooltipRow[] = [
      { label: 'Size', value: row.size === null ? 'Not recorded' : row.size.toString() },
      { label: 'Funds', value: row.funds === null ? 'Not recorded' : fundsText(row.funds) },
      { label: 'Unspent points', value: row.points === null ? 'Not recorded' : row.points.toString() },
      { label: 'Traits unlocked', value: row.traits === null ? 'Not recorded' : row.traits.toString() },
      { label: 'Policy', value: policyText(row), lookupKey: row.policyToken ?? undefined },
    ];
    if (!Chooser) lines.push({ label: 'The game’s policy screen is not available yet', tone: 'muted' });
    return fitTooltip([{ title: row.name ?? NAME_UNAVAILABLE, subtitle: dayLabel(hour), rows: lines }], viewport);
  }, [hour, viewport, Chooser]);
  const open = (row: OrganisationRow) => {
    if (!track || !tag || !Chooser) return;
    setPicked({ kind: 'mioPolicyChooser', sourceId: track.source.id, revision: track.source.revision, branchId: track.branch.id,
      hour, organisation: row.organisation.id });
    dispatch({ type: 'open', breakdown: POLICY.id, hour, playing: runtime.clock.getSnapshot().playing, playhead: widget.time.mode === 'follow' ? hour : null });
  };
  const at = drillHour(drill, hour) ?? hour;

  if (!track) return <WidgetState kind="empty" icon="factory" message="Load a campaign to see organisations." />;
  if (!window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (!tag) return <WidgetState kind="empty" message="Choose a country in widget settings." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading organisations…" />;
  if (!preparedAt(data, hour)) return <WidgetState kind="loading" label="This range is still being prepared." />;
  const country = names.get(nameKey(hour, tag)) ?? tag;
  const header = headerSlot && createPortal(<span className="pill">{rows.length.toLocaleString()} organisations</span>, headerSlot);
  const main = !rows.length ? rosterKnown(data, hour) ? <WidgetState kind="empty" icon="factory" message={`${country} has no industrial organisations at this date.`} />
    : <WidgetState kind="unavailable" message="Industrial organisations were not recorded at this date." />
    : <Table key={`${track.source.id}:${track.branch.id}:${tag}`} label="Industrial organisations" rows={rows} columns={columns} rowKey={row => row.key}
      sort={sort} setOption={setOption} cellBars={options.cellBars === true} density={options.density === 'compact' ? 'compact' : 'comfortable'}
      rowHeight={2 * (tokens.fontSize.title + tokens.fontSize.small)} narrow={narrowTableOf(options)} hoursPerSecond={hoursPerSecond}
      onRowClick={Chooser ? open : undefined} tooltip={tooltip} />;
  return <div ref={anchor} className="industrial-organisations">
    {header}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null && !!picked && !!Chooser} anchor={anchor}
      title={POLICY.label} day={dayLabel(at)} following={drill.follow} canFollow={widget.time.mode === 'follow'}
      onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })} main={main}>
      {picked && Chooser && <Chooser selection={{ ...picked, hour: drill.follow ? hour : at }} active={active} client={runtime.client}
        onDiagnostics={onWindowDiagnostics} />}
    </DrillHost>
  </div>;
}
