import { useCallback, useEffect, useMemo, useState } from 'react';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import { CountryFlagUrl } from '../../components/CountryFlag';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { jumpItem } from '../../charts/jump';
import { runAt, runIndexAt } from '../../charts/series';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedTags } from '../../dashboard/countrySelection';
import { nameKey, useCountryFactsAt } from '../../dashboard/countryNamesAt';
import type { FactSpec } from '../../dashboard/countryFacts';
import { useContentRevision } from '../../dashboard/contentRevision';
import { drawableIcon } from '../../dashboard/artwork';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { readAt } from '../../dashboard/reads';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useProductWindows, useWindowDisplay } from '../../dashboard/productWindows';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { AirAcePortrait, AirAcesManifestReply, AirAcesWindowReply } from '../../generated/dashboard';
import { DEFAULT_SORT, OPTIONS, SORT_CHOICES } from './definition';
import { aceJump, aceName, aceNameValue, aceCallsign, aceRows, aceStatus, aceTooltip, aircraftName, validateAces, validateAcesWindow, validatePortraits, wingName } from './acesData';
import type { AceRow } from './acesData';
import './aces.css';

const EMPTY: readonly AceRow[] = [];
const WARNINGS: string[] = [];
const PORTRAITS: FactSpec<AirAcePortrait> = {
  product: 'air-ace-portraits', perRequest: Infinity,
  params: (branchId, hour, ids) => ({ branchId, hour, ids }),
  extract: value => new Map(validatePortraits(value).portraits.map(p => [p.id, { ...p, image: p.image && drawableIcon(p.image) ? p.image : null }])),
};
const keyOf = (row: AceRow) => row.key;
export default function AcesWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const hourAt = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const speed = usePlaybackSpeed(runtime.clock);
  const viewport = useViewportHeight();
  const content = useContentRevision(runtime.client);
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const filtered = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && filtered);
  const fallbackKey = JSON.stringify(selectedTags(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(fallbackKey), [fallbackKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const window = track && resolveWindow(widget, track, runtime.tracks, hourAt);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const options = resolveOptions(OPTIONS, widget.options);
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const loaded = useProduct<AirAcesManifestReply>(runtime.client, track ? {
    sourceId: track.source.id, revision: track.source.revision, product: 'air-aces', params: { branchId: track.branch.id, view: 'manifest' },
  } : null, active, validateAces);
  const data = loaded.data;
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : null;
  const target = data ? runIndexAt(data.windows, hour) : -1;
  const wanted = useMemo(() => data && target >= 0 ? target + 1 < data.windows.length ? [target, target + 1] : [target] : [], [data, target]);
  const windows = useProductWindows<AirAcesWindowReply>(runtime.client, identity, index => ({
    sourceId: track!.source.id, revision: track!.source.revision, product: 'air-aces',
    params: { branchId: track!.branch.id, view: 'window', window: index },
  }), wanted, active && data !== undefined, validateAcesWindow);
  const current = windows.data.get(target);
  const display = useWindowDisplay(identity, current, hour, runtime.client);
  const pending = current === undefined;
  const recorded = data !== undefined && runAt(data.observed, hour) !== null;
  const read = data && display ? readAt(data.readHours, display.hour) : null;
  const detail = display?.data;
  // Same read and window reuse rows while the clock carries those facts.
  const rows = useMemo(() => data && detail && recorded && read !== null && match.ready ? aceRows(data, detail, Math.max(detail.firstHour, read),
    (tag, read) => tag === null ? widget.countries.kind === 'all' : match.matches(tag, read)) : EMPTY,
  [data, detail, recorded, read, match, widget.countries.kind]);
  const [visible, setVisible] = useState<readonly AceRow[]>(EMPTY);
  useEffect(() => setVisible(EMPTY), [identity]);
  const onVisible = useCallback((rows: readonly AceRow[]) => setVisible(rows), []);
  const countries = useMemo(() => visible.flatMap(row => [row.run.tag, row.run.death?.killerTag].flatMap(tag => tag ? [{ tag, hour: row.hour }] : [])), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, countries, active && data !== undefined);
  const artRequests = useMemo(() => visible.map(row => ({ tag: row.key, hour: row.hour })), [visible]);
  const portraits = useCountryFactsAt(runtime.client, track, artRequests, active && data !== undefined, PORTRAITS, new Set(artRequests.map(item => item.hour)).size);
  const columns = useMemo<readonly TableColumn<AceRow>[]>(() => [
    { key: 'portrait', header: null, label: 'Portrait', sortable: false, width: 'var(--military-portrait-column)', value: () => null, renderMissing: true,
      render: row => { const art = portraits.get(nameKey(row.hour, row.key)); return art?.image ? <img className="aces-portrait" src={art.image} alt="" />
        : <ArtFallback icon="helmet" className="aces-portrait" />; } },
    { ...SORT_CHOICES[1], label: 'Ace', header: 'Ace', width: 'minmax(var(--military-name-min), 1.6fr)', value: aceNameValue, renderMissing: true,
      render: row => { const label = row.run.tag ? labelAt(row.hour)(row.run.tag) : null; return <div className="aces-who"><b title={aceName(row)}>{aceName(row)}</b><small>
        {label ? <CountryFlagUrl className="aces-flag" url={label.flag} reason={label.flagReason} pending={label.flagPending} />
          : <span className="aces-flag aces-country-missing" role="img" aria-label="Country not recorded" data-tooltip="Country not recorded" />}
        {aceCallsign(row) ? `“${aceCallsign(row)}”` : ''}
      </small></div>; } },
    { key: 'wing', header: 'Wing · aircraft', label: 'Wing and aircraft', sortable: false, width: 'minmax(var(--table-name-min), 1fr)', value: () => null, renderMissing: true,
      render: row => <div className="aces-wing"><span title={wingName(row)}>{wingName(row)}</span><small title={aircraftName(row)}>{aircraftName(row)}</small></div> },
    { ...SORT_CHOICES[0], label: 'Status', header: 'Status', width: 'var(--military-status-column)', value: row => row.ace.firstSeen,
      render: row => <small className="aces-status">{aceStatus(row)}</small> },
  ], [portraits, labelAt]);
  const tooltip = useCallback((row: AceRow) => fitTooltip([aceTooltip(row, tag => labelAt(row.hour)(tag).name)], viewport), [labelAt, viewport]);
  const diagnostics = useMemo(() => data ? [...data.warnings, ...(detail?.warnings ?? WARNINGS)] : WARNINGS, [data, detail]);
  useDiagnostics(onDiagnostics, diagnostics);
  if (!track) return <WidgetState kind="empty" icon="star" message="Load a campaign to see its aces." />;
  if (!window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (loaded.error) return <WidgetState kind="error" detail={loaded.error} onRetry={loaded.retry} />;
  if (filtered && groups.error) return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
  if (!data || !match.ready) return <WidgetState kind="loading" label="Loading aces…" />;
  if (match.reason) return <WidgetState kind="unavailable" message={match.reason} />;
  if (!data.available) return <WidgetState kind="unavailable" message="Aces: Not in this recording." />;
  if (!recorded) return <WidgetState kind="unavailable" message="Aces were not recorded at this time." />;
  if (!display) return windows.errors.has(target) ? <WidgetState kind="error" detail={windows.errors.get(target)!} onRetry={windows.retry} /> : <WidgetState kind="loading" label="Loading aces..." />;
  if (!rows.length) return <WidgetState kind="empty" icon="star" message="No aces match these settings." />;
  return <div className="aces-widget">{pending && <div className="aces-loading" role="status">{windows.errors.has(target) ? 'Couldn’t load aces.' : 'Updating…'}{windows.errors.has(target) && <button onClick={windows.retry}>Retry</button>}</div>}<Table key={identity} label="Aces" rows={rows} columns={columns} rowKey={keyOf} sort={sort} setOption={setOption}
    rowHeight={50} cellBars={false} rankMarkers={false} density="comfortable"
    muted={row => !row.run.alive} menu={row => [jumpItem(runtime, track.id, aceJump(row))]} tooltip={tooltip} onVisibleRows={onVisible} hoursPerSecond={speed} /></div>;
}
