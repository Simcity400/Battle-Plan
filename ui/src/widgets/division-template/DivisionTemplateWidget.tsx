import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo, useReducer, useRef, useState, useSyncExternalStore } from 'react';
import { createPortal } from 'react-dom';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import Table from '../../charts/table/Table';
import type { TableColumn, TableTree } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useCountryNameAt, useReachedCountry } from '../../dashboard/reachedProduct';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { useIdentityHour } from '../../dashboard/atHead';
import { usePlaybackHour } from '../../dashboard/follow/useFollow';
import ReachedCountryChoice from '../../dashboard/ReachedCountryChoice';
import { withWindowDiagnostics } from '../../dashboard/gameWindows';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { designerSnapshot, subscribeDesignerSnapshot } from './designerSnapshot';
import { baseWidth, battalionCount, battalionSummary, priorityLabel, templateName, templateRows, templateStructure, templateTooltip, validateDivisionTemplates } from './divisionTemplateData';
import type { TemplateRow } from './divisionTemplateData';
import './divisionTemplate.css';

const NO_LINES: readonly string[] = [];

const rowKey = (row: TemplateRow) => row.key;
const children = (row: TemplateRow) => row.kind === 'obsolete' ? row.children : undefined;
const closedObsolete = () => false;
const TREE: TableTree<TemplateRow> = { children, initiallyOpen: closedObsolete };
const entryOf = (row: TemplateRow) => row.kind === 'template' ? row.entry : null;

export default function DivisionTemplateWidget({ widget, runtime, active, setOption, onDiagnostics, headerSlot }: WidgetProps) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const [drill, dispatch] = useReducer(reduceDrill, CLOSED);
  const chosen = typeof widget.options?.country === 'string' ? [widget.options.country] : [];
  const identityHour = useIdentityHour(runtime.client, track, chosen, runtime.clock, active && widget.time.mode === 'follow',
    clockHour => track ? resolveWindow(widget, track, runtime.tracks, clockHour)?.lastHour ?? null : null);
  const designerHead = usePlaybackHour(runtime.clock, active && drill.breakdown !== null && drill.follow);
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const window = track && resolveWindow(widget, track, runtime.tracks, identityHour ?? runtime.clock.getHour());
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const country = useReachedCountry(widget, runtime, hour, active, widget.options?.country, setOption);
  const tag = country.tag;
  const countryLabel = useCountryNameAt(runtime.client, track, tag ?? country.chosen, hour, active);
  const countryName = countryLabel.name;
  const nameDetail = countryLabel.nameFailure ? <span data-detail={countryLabel.nameFailure.message}>{countryName}</span> : undefined;
  const options = resolveOptions(OPTIONS, widget.options);
  const countFor = options.countFor === 'host' ? 'host' : 'owner';
  const state = useHeadProduct(runtime, widget, 'division-templates', track && window && tag ? { country: countryFromKey(tag), countFor } : null,
    { active, validate: validateDivisionTemplates, structure: templateStructure });
  useShownEachFrame(runtime.clock, state.shownAt, state.live);
  const data = state.data;
  const shownHour = () => state.hour ?? (track ? Math.floor(runtime.clock.getHour()) : hour);
  const rows = useMemo(() => data ? templateRows(data) : [], [data]);
  const snapshot = useSyncExternalStore(subscribeDesignerSnapshot, designerSnapshot, designerSnapshot);
  const viewportHeight = useViewportHeight();
  const anchor = useRef<HTMLDivElement>(null);
  const [opened, setOpened] = useState<{ template: string; designer: 'ordinary' | 'headquarters'; tag: string } | null>(null);
  const [windowLines, setWindowLines] = useState<readonly string[]>(NO_LINES);
  const onWindowDiagnostics = useCallback((lines: string[]) =>
    setWindowLines(previous => previous.length === lines.length && previous.every((line, index) => line === lines[index]) ? previous : lines), []);
  const diagnostics = useMemo(() => data ? withWindowDiagnostics(data.warnings, windowLines) : null, [data, windowLines]);
  useDiagnostics(onDiagnostics, diagnostics);
  useEffect(() => { dispatch({ type: 'close' }); setOpened(null); }, [track?.source.id, track?.source.revision, track?.branch.id, country.chosen]);
  const open = useCallback((row: TemplateRow) => {
    const entry = entryOf(row);
    if (!entry || !snapshot || !data) return;
    setOpened({ template: entry.template, designer: entry.designer ?? 'ordinary', tag: data.tag });
    dispatch({ type: 'open', breakdown: entry.template, hour: shownHour(),
      playing: widget.time.mode === 'follow' && runtime.clock.getSnapshot().playing, playhead: widget.time.mode === 'follow' ? hour : null });
  }, [snapshot, data, widget.time.mode, runtime.clock, hour, state.hour]);
  const columns = useMemo<readonly TableColumn<TemplateRow>[]>(() => [
    { key: 'template', header: 'Template', label: 'Template', hint: 'Sorted by the game’s roster order', width: 'minmax(var(--table-name-min), 2fr)',
      value: row => row.kind === 'obsolete' ? Number.MAX_SAFE_INTEGER : row.entry.rosterOrdinal, renderMissing: true,
      text: row => row.kind === 'obsolete' ? `Obsolete templates (${row.count})` : templateName(row.entry),
      render: row => row.kind === 'obsolete' ? `Obsolete templates (${row.count})`
        : <span className="division-template-name" data-tooltip={row.entry.template}>
          {row.entry.icon ? <img src={row.entry.icon} alt="" /> : <ArtFallback icon="helmet" />}
          <span>{templateName(row.entry)}</span></span> },
    { key: 'fielded', header: 'Fielded', label: 'Fielded', numeric: true, width: 'max-content', bar: 'visible',
      value: row => entryOf(row)?.fielded ?? null, renderMissing: true,
      render: row => row.kind === 'obsolete' ? '' : row.entry.fielded === null ? <span className="division-template-missing" data-tooltip="Not recorded">—</span> : row.entry.fielded },
    { key: 'battalions', header: 'Battalions', label: 'Battalions', width: 'minmax(calc(var(--space-6) * 6), 3fr)', priority: 1, inTooltip: true,
      value: row => { const entry = entryOf(row); return entry ? battalionCount(entry) : null; }, renderMissing: true,
      render: row => { const entry = entryOf(row); if (!entry) return ''; const summary = battalionSummary(entry);
        return summary === null ? <span className="division-template-missing" data-tooltip="Not recorded">—</span> : <span title={summary}>{summary}</span>; } },
    { key: 'width', header: 'Base combat width', label: 'Base combat width', numeric: true, width: 'max-content', priority: 2, inTooltip: true,
      value: row => { const raw = entryOf(row)?.baseWidthRaw; return raw == null ? null : BigInt(raw); },
      text: row => { const entry = entryOf(row); return entry ? baseWidth(entry) : null; },
      render: row => { const entry = entryOf(row); return entry ? baseWidth(entry) : ''; } },
    { key: 'priority', header: 'Priority', label: 'Priority', width: 'max-content', priority: 3, inTooltip: true,
      value: row => entryOf(row)?.priority ?? null, renderMissing: true,
      text: row => { const entry = entryOf(row); return entry ? priorityLabel(entry) ?? '—' : ''; },
      render: row => { const entry = entryOf(row); if (!entry) return '';
        const label = priorityLabel(entry); return label ?? <span className="division-template-missing" data-tooltip={entry.priorityMissing ?? 'Not recorded'}>—</span>; } },
  ], []);
  const tooltip = useCallback((row: TemplateRow) => row.kind === 'obsolete' || !data ? { title: `Obsolete templates (${row.kind === 'obsolete' ? row.count : 0})`, rows: [] }
    : fitTooltip([templateTooltip(row.entry, shownHour(), !!snapshot)], viewportHeight),
  [data, snapshot, viewportHeight, state.hour]);
  const current = data ? data.templates.filter(entry => !entry.obsolete).length : 0;
  const pill = headerSlot && data && createPortal(<span className="pill">{current} {current === 1 ? 'template' : 'templates'}</span>, headerSlot);
  let main;
  if (!track) main = <WidgetState kind="empty" message="Load a campaign to see templates." />;
  else if (!window) main = <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  else if (!country.chosen) main = <WidgetState kind="empty" message="Choose a country to see its templates." />;
  else if (country.error && !country.historyHeld) main = <WidgetState kind="error" failure={country.error} onRetry={country.retry} />;
  else if (country.holders) main = <ReachedCountryChoice tag={country.chosen} holders={country.holders} onPick={country.pick} />;
  else if (country.reason) main = <WidgetState kind="unavailable" message={country.reason} />;
  else if (country.absent) main = <WidgetState kind="unavailable" message={nameDetail ? 'This country does not exist at this date.' : `${countryName} does not exist at this date.`} detail={nameDetail} />;
  else if (!tag) main = <WidgetState kind="loading" label="Loading templates…" />;
  else if (state.error) main = <WidgetState kind="error" failure={state.error} onRetry={state.retry} />;
  else if (!data) main = <WidgetState kind="loading" label="Loading templates…" />;
  else if (!rows.length) main = <WidgetState kind="empty" message={nameDetail ? 'No division templates at this date.' : `${countryName} has no division templates at this date.`} detail={nameDetail} />;
  else main = <Table rowRevision={row => [row, columns]} label="Division templates" rows={rows} columns={columns} rowKey={rowKey} tree={TREE}
    sort={tableSortOf(options.tableSort, DEFAULT_SORT)} setOption={setOption} cellBars={options.cellBars === true}
    rankMarkers={false} density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)}
    onRowClick={snapshot ? open : undefined} tooltip={tooltip} hoursPerSecond={hoursPerSecond} />;
  const draw = snapshot;
  const pickedHour = drillHour(drill, Math.floor(designerHead) + (track?.axisOffset ?? 0)) ?? hour;
  const root = useCallback((element: HTMLDivElement | null) => { anchor.current = element; state.root(element); }, [state.root]);
  return <div ref={root} className="division-template" data-detail={countryLabel.nameFailure?.message}>{pill}
    { }
    {country.chosen && country.error && country.historyHeld ? <WidgetState kind="error" failure={country.error} onRetry={country.retry} /> : null}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null && !!draw && !!opened}
      title="Division designer" day={formatDateTime(pickedHour)} following={drill.follow} canFollow anchor={anchor}
      onClose={() => { dispatch({ type: 'close' }); setOpened(null); setWindowLines(NO_LINES); }} onFollow={() => dispatch({ type: 'follow' })} main={main}>
      {draw && opened && track && draw({ client: runtime.client, source: track.source, branchId: track.branch.id, hour: pickedHour,
        tag: opened.tag, template: opened.template, designer: opened.designer, onDiagnostics: onWindowDiagnostics }, active)}
    </DrillHost></div>;
}
