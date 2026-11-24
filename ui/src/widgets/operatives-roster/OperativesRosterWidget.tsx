import { countryFromKey, isSlotKey } from '../../dashboard/countryKey.ts';
import { keyNameAt, useSlotTags } from '../../dashboard/countrySlots.ts';
import { useCallback, useEffect, useMemo, useRef } from 'react';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import { jumpItem } from '../../charts/jump';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { fitTooltip } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, noMembersAt, resolveCountries } from '../../dashboard/groups';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import NoMembersState from '../../dashboard/NoMembersState';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import type { OperativeTenuresReply, OperativesDepartedReply, OperativesRosterReply } from '../../generated/dashboard';
import { DEFAULT_SORT, definition, OPTIONS } from './definition';
import { capturedText, departureText, labelHour, operativeMoments, operativeName, operativeTooltip, paintedOf, paintedRows, paintedText, paintRoster,
  rosterRows, rosterStructure, validateDeparted, validateRoster, validateTenures } from './rosterData';
import type { OperativeRow, Painted, PaintedField } from './rosterData';
import './operativesRoster.css';
import { useChangedState } from '../../dashboard/changedState';
import { productRequest } from '../../dashboard/request.ts';

const NO_ROWS: OperativeRow[] = [];
const NO_WARNINGS: string[] = [];
const rowKey = (row: OperativeRow) => row.key;
const muted = (row: OperativeRow) => row.departed !== null;
const sameCountries = (a: OperativeRow, b: OperativeRow) => a.key === b.key && a.departed?.lastHour === b.departed?.lastHour
  && a.operative.roster === b.operative.roster && a.operative.owner.tag === b.operative.owner.tag
  && a.operative.target?.tag === b.operative.target?.tag && a.operative.capturer?.tag === b.operative.capturer?.tag;
const sameCountryRows = (held: readonly OperativeRow[], next: readonly OperativeRow[]) => held.length === next.length
  && held.every((row, index) => sameCountries(row, next[index]));

export default function OperativesRosterWidget({ widget, runtime, active, setOption, onDiagnostics }: WidgetProps) {
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const viewportHeight = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const lastHour = track?.branch.lastHour ?? 0;
  const groups = useCountryGroups(runtime, track, active);
  const fallback = defaultCountryKey(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour, definition.countries.max),
    [widget.countries, fallback, groups.data, lastHour]);
  const match = useMemo(() => countryMatch(widget.countries, fallback ? [fallback] : [], groups.data, lastHour), [widget.countries, fallback, groups.data, lastHour]);
  const tagsKey = JSON.stringify(resolved.tags);
  const params = useMemo(() => track && resolved.tags.length ? { countries: resolved.tags.map(countryFromKey) } : null, [track?.branch.id, tagsKey]);
  const root = useRef<HTMLDivElement | null>(null);
  const painted = useRef<Map<string, Painted>>(new Map());
  const paint = useCallback((reply: OperativesRosterReply) => {
    painted.current = paintedRows(reply);
    paintRoster(root.current, painted.current);
  }, []);
  const gate = useGate(runtime, widget);
  const head = useHeadProduct(runtime, gate, 'operatives-roster', params, { active, validate: validateRoster, structure: rosterStructure, paint });
  if (head.data && head.read !== 'followed') painted.current = paintedRows(head.data);
  useShownEachFrame(runtime.clock, () => head.shownAt(), head.live);
  const hour = head.at ?? head.hour ?? lastHour;
  const unheldSlots = useSlotTags(runtime.client, track, active && !!head.data?.unheld.some(isSlotKey));
  const departedAt = widget.time.mode === 'campaign' && options.departed === true && head.read === 'asked' ? gate.recorded(head.hour) : null;
  const departed = useProduct<OperativesDepartedReply>(runtime.client, track && params && departedAt !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'operatives-departed', params: { branchId: track.branch.id, countries: params.countries, hour: departedAt } }) : null, active, validateDeparted, track?.source);
  const tenureHour = useRef<number | null>(null);
  const tenureAt = gate.recorded(tenureHour.current);
  const tenuresAsked = useProduct<OperativeTenuresReply>(runtime.client, track && params && tenureAt !== null ? productRequest({ sourceId: track.source.id,
    revision: track.source.revision, product: 'operative-tenures', params: { branchId: track.branch.id, countries: params.countries, hour: tenureAt } }) : null,
    active, validateTenures, track?.source);
  const tenures = tenuresAsked.data;
  if (head.data && (tenureHour.current === null || (tenures && (hour < tenures.firstHour || hour > tenures.lastHour)))) tenureHour.current = hour;
  const rows = useMemo(() => match.ready ? rosterRows(head.data, departedAt !== null ? departed.data : undefined, tenures, hour, match.matches) : NO_ROWS,
    [head.data, departed.data, departedAt, tenures, hour, match]);
  const [visible, onVisibleRows] = useChangedState<readonly OperativeRow[]>(NO_ROWS, sameCountryRows);
  const wanted = useMemo(() => {
    const unique = new Map<string, { tag: string; hour: number }>();
    for (const row of visible) {
      const at = labelHour(row, hour);
      const operative = row.operative;
      for (const tag of [operative.roster, operative.owner.tag, operative.target?.tag, operative.capturer?.tag]) {
        if (tag) unique.set(JSON.stringify([at, tag]), { tag, hour: at });
      }
    }
    return [...unique.values()];
  }, [visible, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const comparison = useMemo(() => new Set(rows.map(row => row.operative.owner.tag ?? `#${row.operative.owner.id}`)).size > 1, [rows]);
  const columns = useMemo<readonly TableColumn<OperativeRow>[]>(() => {
    const painting = (row: OperativeRow, field: PaintedField) => paintedText(row.departed ? paintedOf(row.operative) : painted.current.get(row.key), field);
    const meter = (row: OperativeRow, field: PaintedField, label: string) => <div className={`progress-meter operative-meter is-${row.departed ? 'idle' : 'live'}`} role="meter" aria-label={label}
      data-op-row={row.departed ? undefined : row.key} data-op-field={field}><i style={{ width: painting(row, field) }} /></div>;
    const live = (row: OperativeRow, field: PaintedField) => row.departed ? {} : { 'data-op-row': row.key, 'data-op-field': field };
    return [
      { key: 'name', header: 'Operative', label: 'Operative', renderMissing: true, width: 'minmax(calc(var(--space-6) * 8), 2fr)', value: row => operativeName(row.operative),
        render: row => {
          const operative = row.operative;
          const owner = operative.owner.tag ? labelAt(labelHour(row, hour))(operative.owner.tag) : null;
          const departure = departureText(row);
          return <span className="operative-person">{operative.portrait !== null ? <img src={row.artwork[operative.portrait]} alt="" /> : <ArtFallback icon="helmet" name={operativeName(operative)} />}
            <span className="operative-cell-detail"><span>{comparison && owner
              ? <CountryCell name={operativeName(operative)} flag={owner.flag} flagReason={owner.flagReason} flagPending={owner.flagPending} /> : operativeName(operative)}</span>
              {departure && <small>{departure}</small>}
            </span>
          </span>;
        } },
      { key: 'skill', header: 'Skill', label: 'Skill', numeric: true, bar: 'visible', priority: 2, width: 'minmax(calc(var(--space-6) * 3), 1fr)', value: row => row.operative.skill },
      { key: 'experience', header: 'Experience', label: 'Experience', numeric: true, renderMissing: true, priority: 3, width: 'minmax(calc(var(--space-6) * 5), 1fr)',
        text: row => painting(row, 'experience'), value: row => (row.departed ? paintedOf(row.operative) : painted.current.get(row.key))?.experienceRaw ?? null,
        render: row => <span className="operative-experience"><span {...live(row, 'experience')}>{painting(row, 'experience')}</span>
          {options.cellBars === true && row.operative.experienceRatio !== null && meter(row, 'experienceMeter', 'Operative experience')}
        </span> },
      { key: 'assignment', header: 'Assignment', label: 'Assignment', renderMissing: true, priority: 1, text: row => row.operative.assignmentName ?? 'Unknown',
        width: 'minmax(calc(var(--space-6) * 9), 2fr)', value: row => row.operative.assignmentName,
        render: row => {
          const operative = row.operative;
          return <span className="operative-assignment">{operative.assignmentIcon !== null && <img src={row.artwork[operative.assignmentIcon]} alt="" />}
            <span className="operative-cell-detail"><span>{operative.assignmentName ?? 'Unknown'}</span>
              {operative.assignmentDuration !== null && <small {...live(row, 'duration')}>{painting(row, 'duration')}</small>}
              {options.cellBars === true && operative.assignmentProgress !== null && meter(row, 'progress', 'Assignment progress')}
              {operative.target?.tag && <CountryCell {...labelAt(labelHour(row, hour))(operative.target.tag)} />}
            </span>
          </span>;
        } },
      { key: 'captured', header: 'Captured', label: 'Captured', renderMissing: true, priority: 4, width: 'minmax(calc(var(--space-6) * 8), 2fr)',
        text: row => capturedText(row.operative, tag => labelAt(labelHour(row, hour))(tag).name), value: row => Number(row.operative.captured),
        render: row => {
          const operative = row.operative;
          if (!operative.captured) return 'No';
          return <span className="operative-cell-detail"><span>{operative.capturer?.tag ? <CountryCell {...labelAt(labelHour(row, hour))(operative.capturer.tag)} /> : 'Yes · capturer unknown'}</span>
            <small>{operative.capturedHour === null ? 'Capture date unknown' : formatDateTime(operative.capturedHour)}</small>
          </span>;
        } },
    ];
  }, [comparison, labelAt, hour, options.cellBars]);
  const tooltip = useCallback((row: OperativeRow) => fitTooltip([operativeTooltip(row, row.departed ? undefined : painted.current.get(row.key), hour,
    (tag, at) => labelAt(at)(tag))], viewportHeight), [hour, labelAt, viewportHeight]);
  useEffect(() => { paintRoster(root.current, painted.current); });
  useEffect(() => {
    onDiagnostics(head.data ? [...head.data.warnings, ...departedAt !== null ? departed.data?.warnings ?? [] : []] : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [head.data, departed.data, departedAt, onDiagnostics]);
  const ref = useCallback((element: HTMLDivElement | null) => { root.current = element; head.root(element); }, [head.root]);

  const none = noMembersAt(widget.countries, resolved, groups.data, hour, lastHour);
  const data = head.data;
  let body;
  if (!track) body = <WidgetState kind="empty" icon="shield" message="Load a campaign and choose a country to see its operatives." />;
  else if (head.outside) body = <WidgetState kind="outside" read={head} />;
  else if (head.error) body = <WidgetState kind="error" failure={head.error} onRetry={head.retry} />;
  else if (groups.error && !groups.data) body = <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} />;
  else if (!groups.data) body = <WidgetState kind="loading" label="Loading operatives…" />;
  else if (resolved.reason) body = <WidgetState kind="unavailable" message={resolved.reason} />;
  else if (match.reason) body = <WidgetState kind="unavailable" message={match.reason} />;
  else if (none) body = <NoMembersState reason={none.reason} />;
  else if (!data) body = <WidgetState kind="loading" label="Loading operatives roster…" />;
  else if (!rows.length && data.agencyFeature === false) body = <WidgetState kind="unavailable" message="Intelligence agencies unavailable." />;
  else if (!rows.length && data.missing) body = <WidgetState kind="unavailable" message="Operatives weren’t recorded at this time." detail={data.missing} />;
  else if (!rows.length && data.unheld.length === resolved.tags.length) body = <WidgetState kind="unavailable" message="Operatives weren’t recorded at this time." detail={`No country holds ${data.unheld.map(key => keyNameAt(unheldSlots, key, hour) ?? 'Unknown').join(', ')} at this time.`} />;
  else if (!rows.length) body = <WidgetState kind="empty" message="No operatives in the selected roster." />;
  else body = <Table rowRevision={row => [row, columns]} label="Operatives roster" rows={rows} columns={columns} rowKey={rowKey}
    sort={tableSortOf(options.tableSort, DEFAULT_SORT)} setOption={setOption}
    cellBars={options.cellBars === true} rank={options.showRank === true} rankMarkers={options.rankMarkers === true}
    density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)} rowHeight={2 * (tokens.fontSize.title + tokens.fontSize.small)} muted={muted}
    menu={row => operativeMoments(row).map(moment => ({ ...jumpItem(runtime, track.id, moment.hour), label: moment.label }))}
    tooltip={tooltip} onVisibleRows={onVisibleRows} hoursPerSecond={hoursPerSecond} />;
  return <div ref={ref} className="operatives-roster">
    {!!rows.length && data && data.unheld.length > 0 && <p className="operative-note">{data.unheld.length} country rosters unavailable.</p>}
    {departed.error && departedAt !== null && <WidgetState kind="error" what="departed operatives" failure={departed.error} onRetry={departed.retry} />}
    {groups.error && groups.data ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null}
    {body}
  </div>;
}
