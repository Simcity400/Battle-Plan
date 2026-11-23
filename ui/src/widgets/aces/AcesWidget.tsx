import { useCallback, useEffect, useMemo, useState } from 'react';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import { CountryFlagUrl } from '../../components/CountryFlag';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import { useViewportHeight } from '../../charts/useChartTokens';
import { jumpItem } from '../../charts/jump';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import { nameKey, useCountryFactsAt } from '../../dashboard/countryNamesAt';
import type { FactSpec } from '../../dashboard/countryFacts';
import { useContentRevision } from '../../dashboard/contentRevision';
import { drawableIcon } from '../../dashboard/artwork';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { headState } from '../../dashboard/HeadState';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { DEFAULT_SORT, OPTIONS, SORT_CHOICES } from './definition';
import { aceCallsign, aceDate, aceJump, aceName, aceNameValue, aceRows, aceStatus, aceTooltip, aircraftName, validateAces, validatePortraits, wingName } from './acesData';
import type { AceRow } from './acesData';
import './aces.css';
import type { AirAcePortrait, AirAcesReply } from '../../generated/dashboard';

const EMPTY: readonly AceRow[] = [];
const PORTRAITS: FactSpec<AirAcePortrait> = {
  product: 'air-ace-portraits', perRequest: Infinity, shared: true,
  params: (branchId, hour, ids) => ({ branchId, hour, ids }),
  extract: value => new Map(validatePortraits(value).portraits.map(p => [p.id, { ...p, image: p.image && drawableIcon(p.image) ? p.image : null }])),
};
const PARAMS = {};
const keyOf = (row: AceRow) => row.key;
const structure = (reply: AirAcesReply) => JSON.stringify(reply);

export default function AcesWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const speed = usePlaybackMotion(runtime.clock);
  const viewport = useViewportHeight();
  const content = useContentRevision(runtime.client);
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const filtered = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && filtered);
  const fallbackKey = JSON.stringify(selectedCountryKeys(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(fallbackKey), [fallbackKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const options = resolveOptions(OPTIONS, widget.options);
  const head = useHeadProduct(runtime, useGate(runtime, widget), 'air-aces', track ? PARAMS : null, { active, validate: validateAces, structure });
  const { data, error, retry, root, read: reading, shownAt, live: shownLive } = head;
  useShownEachFrame(runtime.clock, shownAt, shownLive);
  const identity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, content]) : null;
  const read = data?.readHour ?? null;
  const sort = tableSortOf(options.tableSort, DEFAULT_SORT);
  const rows = useMemo(() => data && read !== null && match.ready ? aceRows(data, read,
    (tag, at) => tag === null ? widget.countries.kind === 'all' : match.matches(tag, at)) : EMPTY,
  [data, read, match, widget.countries.kind]);
  const [visible, setVisible] = useState<readonly AceRow[]>(EMPTY);
  useEffect(() => setVisible(EMPTY), [identity]);
  const onVisible = useCallback((rows: readonly AceRow[]) => setVisible(rows), []);
  const countries = useMemo(() => visible.flatMap(row => [row.ace.tag, row.ace.death?.killerTag].flatMap(tag => tag ? [{ tag, hour: row.hour }] : [])), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, countries, active && data !== undefined);
  const artRequests = useMemo(() => visible.map(row => ({ tag: row.key, hour: row.hour })), [visible]);
  const portraits = useCountryFactsAt(runtime.client, track, artRequests, active && data !== undefined, PORTRAITS, new Set(artRequests.map(item => item.hour)).size);
  const columns = useMemo<readonly TableColumn<AceRow>[]>(() => [
    { key: 'portrait', header: null, label: 'Portrait', sortable: false, width: 'var(--military-portrait-column)', value: () => null, renderMissing: true,
      render: row => { const art = portraits.get(nameKey(row.hour, row.key)); return art && !(art instanceof Error) && art.image ? <img className="aces-portrait" src={art.image} alt="" />
        : <ArtFallback icon="helmet" className="aces-portrait" failure={art instanceof Error ? art : undefined} />; } },
    { ...SORT_CHOICES[1], label: 'Ace', header: 'Ace', width: 'minmax(var(--military-name-min), 1.6fr)', value: aceNameValue, renderMissing: true,
      render: row => { const label = row.ace.tag ? labelAt(row.hour)(row.ace.tag) : null; return <div className="aces-who"><b title={aceName(row)}>{aceName(row)}</b><small>
        {label ? <CountryFlagUrl className="aces-flag" url={label.flag} detail={label.flagDetail} reason={label.flagReason} pending={label.flagPending} />
          : <span className="aces-flag aces-country-missing" role="img" aria-label="Country not recorded" data-tooltip="Country not recorded" />}
        {aceCallsign(row) ? `“${aceCallsign(row)}”` : ''}
      </small></div>; } },
    { key: 'wing', header: 'Wing · aircraft', label: 'Wing and aircraft', sortable: false, width: 'minmax(var(--table-name-min), 1fr)', priority: 2, inTooltip: true, value: () => null, renderMissing: true,
      render: row => <div className="aces-wing"><span title={wingName(row)}>{wingName(row)}</span><small title={aircraftName(row)}>{aircraftName(row)}</small></div> },
    { ...SORT_CHOICES[0], label: 'Status', header: 'Status', width: 'var(--military-status-column)', priority: 1, inTooltip: true, value: aceDate, renderMissing: true,
      render: row => <small className="aces-status">{aceStatus(row)}</small> },
  ], [portraits, labelAt]);
  const tooltip = useCallback((row: AceRow) => fitTooltip([{ ...aceTooltip(row, tag => labelAt(row.hour)(tag).name),
    ...countryNameDetails(...[row.ace.tag, row.ace.death?.killerTag].map(tag => tag == null ? null : labelAt(row.hour)(tag))) }], viewport), [labelAt, viewport]);
  const diagnostics = useMemo(() => [...data?.warnings ?? [], ...[...portraits.values()].filter((value): value is Error => value instanceof Error).map(error => error.message)], [data, portraits]);
  useDiagnostics(onDiagnostics, data ? diagnostics : null);
  if (!track) return <WidgetState kind="empty" icon="star" message="Load a campaign to see its aces." />;
  const shared = headState(head);
  if (shared) return <div ref={root}>{shared}</div>;
  if (reading === 'off' && !data && !error) return <div ref={root}><WidgetState kind="outside" /></div>;
  if (error && !data) return <div ref={root}><WidgetState kind="error" failure={error} onRetry={retry} /></div>;
  const groupFailure = filtered && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return <div ref={root}>{groupFailure}</div>;
  if (!data || !match.ready) return <div ref={root}>{groupFailure}<WidgetState kind="loading" label="Loading aces…" /></div>;
  if (match.reason) return <div ref={root}>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></div>;
  if (data.missing) return <div ref={root}>{groupFailure}<WidgetState kind="unavailable" message={data.missing} /></div>;
  if (!rows.length) return <div ref={root}>{groupFailure}<WidgetState kind="empty" icon="star" message="No aces match these settings." /></div>;
  return <div ref={root} className="aces-widget">{groupFailure}<Table key={identity} label="Aces" rows={rows} columns={columns} rowKey={keyOf} sort={sort} setOption={setOption}
    rowHeight={50} cellBars={false} rankMarkers={false} density="comfortable" narrow={narrowTableOf(options)}
    muted={row => !row.ace.alive} menu={row => [jumpItem(runtime, track.id, aceJump(row))]} tooltip={tooltip} onVisibleRows={onVisible} hoursPerSecond={speed} /></div>;
}
