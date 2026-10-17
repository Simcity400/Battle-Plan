import { pendingAt, preparedPrefix, fieldPreparing } from '../../dashboard/preparedPrefix';
import ArtFallback from '../../components/ArtFallback';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { tableSortOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { runAt } from '../../charts/series';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { countryIdentityIndex } from '../../dashboard/countryIdentity';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, resolveCountries } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useParityRows } from '../../dashboard/parityRows';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { OperativeRosterReply } from '../../generated/dashboard';
import { DEFAULT_SORT, definition, OPTIONS } from './definition';
import { departureText, operativeLabelHour, operativeMoments, operativeName, operativeTooltip, rosterAt, validateOperativesRosterReply } from './operativesRosterData';
import type { OperativeRow } from './operativesRosterData';
import './operativesRoster.css';

const EMPTY_ROWS: OperativeRow[] = [];
const EMPTY_WARNINGS: string[] = [];
const rowKey = (row: OperativeRow) => row.key;
const muted = (row: OperativeRow) => row.departed;

export default function OperativesRosterWidget({ widget, runtime, active, setOption, onDiagnostics }: WidgetProps) {
  const playbackHour = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const tokens = useChartTokens();
  const viewportHeight = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(track => track.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data,
    track?.branch.lastHour ?? hour, definition.countries.max), [widget.countries, fallback, groups.data, track, hour]);
  const first = track ? track.branch.firstHour + Math.floor((hour - track.branch.firstHour) / 24) * 24 : hour;
  const request = track && resolved.tags.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'operatives-roster',
    params: runtime.client.preparedQuery
      ? { branchId: track.branch.id, tags: resolved.tags, firstHour: first, lastHour: Math.min(first + 23, track.branch.lastHour), includeDeparted: widget.time.mode === 'campaign' && options.departed === true }
      : { branchId: track.branch.id, tags: resolved.tags } } : null;
  const { data, error, retry } = useProduct<OperativeRosterReply>(runtime.client, request, active, validateOperativesRosterReply, track?.source);
  const match = useMemo(() => countryMatch(widget.countries, fallback ? [fallback] : [], groups.data,
    track?.branch.lastHour ?? hour), [widget.countries, fallback, groups.data, track?.branch.lastHour, hour]);
  const membershipUnknown = resolved.membership !== null && resolved.unobserved.some(span => span.firstHour <= hour && hour <= span.lastHour);
  const view = useMemo(() => data && match.ready ? rosterAt(data, hour, widget.time.mode === 'campaign' && options.departed === true,
    match.matches) : null, [data, hour, widget.time.mode, options.departed, match]);
  const rows = view?.rows ?? EMPTY_ROWS;
  const root = useRef<HTMLDivElement>(null);
  useParityRows(root, () => ({ kind: 'table', label: 'Operative roster values', complete: Boolean(data && view && match.ready), total: rows.length,
    rows: rows.map(row => ({ key: row.key, country: row.country, joinedHour: row.entry.joinedHour, departedHour: row.departed ? row.entry.departedHour : null,
      value: row.run.value, readHour: row.readHour, assignmentReadHour: row.assignmentReadHour, moments: operativeMoments(row) })),
  }));
  const [visibleRows, setVisibleRows] = useState<readonly OperativeRow[]>(EMPTY_ROWS);
  const onVisibleRows = useCallback((rows: readonly OperativeRow[]) => setVisibleRows(rows), []);
  const wanted = useMemo(() => {
    const unique = new Map<string, { tag: string; hour: number }>();
    for (const row of visibleRows) {
      const labelHour = operativeLabelHour(row, hour);
      for (const tag of [row.country, row.run.value.ownerTag, row.run.value.targetTag, row.run.value.capturerTag]) {
        if (tag !== null) unique.set(JSON.stringify([labelHour, tag]), { tag, hour: labelHour });
      }
    }
    return [...unique.values()];
  }, [visibleRows, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const comparison = useMemo(() => {
    if (!groups.data) return false;
    const identities = countryIdentityIndex(groups.data.countryIdentities ?? []);
    const owners = new Set<string>();
    for (const row of rows) {
      const tag = row.run.value.ownerTag;
      const key = tag === null ? null : identities.keyAt(tag, operativeLabelHour(row, hour));
      if (key !== null) owners.add(key);
      if (owners.size > 1) return true;
    }
    return false;
  }, [groups.data, rows, hour]);
  const columns = useMemo<readonly TableColumn<OperativeRow>[]>(() => [
    { key: 'name', header: 'Operative', label: 'Operative', renderMissing: true,
      width: 'minmax(calc(var(--space-6) * 8), 2fr)', value: row => row.run.value.name,
      render: row => {
        const value = row.run.value;
        const owner = value.ownerTag === null ? null : labelAt(operativeLabelHour(row, hour))(value.ownerTag);
        const departure = departureText(row);
        return <span className="operative-person">{value.portrait !== null && data ? <img src={data.artwork[value.portrait]} alt="" /> : <ArtFallback icon="helmet" />}
          <span className="operative-cell-detail"><span>{comparison && owner !== null
            ? <CountryCell name={operativeName(value)} flag={owner.flag} flagReason={owner.flagReason} flagPending={owner.flagPending} /> : operativeName(value)}</span>
            {departure && <small>{departure}</small>}
          </span>
        </span>;
      } },
    { key: 'skill', header: 'Skill', label: 'Skill', numeric: true, bar: 'visible',
      width: 'minmax(calc(var(--space-6) * 3), 1fr)', value: row => row.run.value.skill },
    { key: 'experience', header: 'Experience', label: 'Experience', numeric: true, renderMissing: true,
      width: 'minmax(calc(var(--space-6) * 5), 1fr)', value: row => BigInt(row.run.value.experienceRaw),
      render: row => <span className="operative-experience"><span>{row.run.value.experienceText ?? 'Unknown'}</span>
        {options.cellBars === true && row.run.value.experienceRatio !== null && <ProgressMeter value={row.run.value.experienceRatio} state={row.departed ? 'idle' : 'live'} label="Operative experience" />}
      </span> },
    { key: 'assignment', header: 'Assignment', label: 'Assignment', renderMissing: true,
      width: 'minmax(calc(var(--space-6) * 9), 2fr)', value: row => row.run.value.assignmentName,
      render: row => {
        const value = row.run.value;
        return <span className="operative-assignment">{value.assignmentIcon !== null && data && <img src={data.artwork[value.assignmentIcon]} alt="" />}
          <span className="operative-cell-detail"><span>{value.assignmentName ?? 'Unknown'}</span>
            {value.assignmentDuration !== null && <small>{value.assignmentDuration}</small>}
            {options.cellBars === true && value.assignmentProgress !== null && <ProgressMeter value={value.assignmentProgress} state={row.departed ? 'idle' : 'live'} label="Assignment progress" />}
            {value.targetTag !== null && <CountryCell {...labelAt(operativeLabelHour(row, hour))(value.targetTag)} />}
          </span>
        </span>;
      } },
    { key: 'captured', header: 'Captured', label: 'Captured', renderMissing: true,
      width: 'minmax(calc(var(--space-6) * 8), 2fr)', value: row => row.run.value.captured === null ? null : Number(row.run.value.captured),
      render: row => {
        const value = row.run.value;
        if (value.captured !== true) return value.captured === null ? 'Unknown' : 'No';
        return <span className="operative-cell-detail"><span>{value.capturerTag === null
          ? 'Yes · capturer unknown' : <CountryCell {...labelAt(operativeLabelHour(row, hour))(value.capturerTag)} />}</span>
          <small>{value.capturedHour === null ? 'Capture date unknown' : formatDateTime(value.capturedHour)}</small>
        </span>;
      } },
  ], [comparison, data, labelAt, hour, options.cellBars]);
  const tooltip = useCallback((row: OperativeRow) => fitTooltip(data ? [operativeTooltip(data, row, hour,
    (tag, at) => labelAt(at)(tag))] : [], viewportHeight), [data, hour, labelAt, viewportHeight]);
  useEffect(() => {
    onDiagnostics(data?.warnings ?? EMPTY_WARNINGS);
    return () => onDiagnostics(EMPTY_WARNINGS);
  }, [data, onDiagnostics]);

  let body;
  if (!track) body = <WidgetState kind="empty" message="Load a campaign and choose a country to see its operatives." />;
  else if (!window) body = <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  else if (error) body = <WidgetState kind="error" detail={error} onRetry={retry} />;
  else if (groups.error) body = <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
  else if (!groups.data) body = <WidgetState kind="loading" label="Loading operatives…" />;
  else if (resolved.reason) body = <WidgetState kind="unavailable" message={resolved.reason} />;
  else if (match.reason) body = <WidgetState kind="unavailable" message={match.reason} />;
  else if (membershipUnknown) body = <WidgetState kind="unavailable" message="This group's members were not recorded at this moment." />;
  else if (!data || !view) body = <WidgetState kind="loading" label="Loading operatives roster…" />;
  else if (pendingAt(preparedPrefix(data), hour)) body = <WidgetState kind="loading" label="This range is still being prepared." />;
  else if (!rows.length && runAt(data.agencyAbsent, hour)) body = <WidgetState kind="unavailable" message="Intelligence agencies unavailable." />;
  else if (!rows.length && view.unknown.length) body = <WidgetState kind="unavailable" message="Operatives weren’t recorded at this time." />;
  else if (!rows.length) body = <WidgetState kind="empty" message="No operatives in the selected roster." />;
  else body = <Table label="Operatives roster" rows={rows} columns={columns} rowKey={rowKey}
      sort={tableSortOf(options.tableSort, DEFAULT_SORT)} setOption={setOption}
      cellBars={options.cellBars === true} rank={options.showRank === true} rankMarkers={options.rankMarkers === true}
      density={options.density === 'compact' ? 'compact' : 'comfortable'} rowHeight={2 * (tokens.fontSize.title + tokens.fontSize.small)} muted={muted}
      menu={row => operativeMoments(row).map(moment => ({ ...jumpItem(runtime, track.id, moment.hour), label: moment.label }))}
      tooltip={tooltip} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} />;
  return <div ref={root} className="operatives-roster">
    {!!rows.length && view && view.unknown.length > 0 && <p className="operative-note">{view.unknown.length} country rosters unavailable.</p>}
    {data?.countries.some(country => country.operatives.some(entry => fieldPreparing(entry, 'joinedHour'))) && <p className="chart-line-key">First presence, departure and earlier departed operatives are still being prepared.</p>}
    {body}
  </div>;
}
