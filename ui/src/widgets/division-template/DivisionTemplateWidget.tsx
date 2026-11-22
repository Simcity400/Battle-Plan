import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useCallback, useEffect, useMemo,  useRef, useState, useSyncExternalStore } from 'react';
import { createPortal } from 'react-dom';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import Table from '../../charts/table/Table';
import type { TableColumn, TableTree } from '../../charts/table/Table';
import { narrowTableOf, tableSortOf } from '../../charts/table/options';
import { fitTooltip } from '../../charts/tooltip';
import { useViewportHeight } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useCountryNameAt } from '../../dashboard/reachedProduct';
import GateState from '../../dashboard/GateState';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { withWindowDiagnostics } from '../../dashboard/gameWindows';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { DEFAULT_SORT, OPTIONS } from './definition';
import { designerSnapshot, subscribeDesignerSnapshot } from './designerSnapshot';
import { baseWidth, battalionCount, battalionSummary, priorityLabel, templateName, templateRows, templateStructure, templateTooltip, validateDivisionTemplates } from './divisionTemplateData';
import type { TemplateRow } from './divisionTemplateData';
import './divisionTemplate.css';
import { SPEC } from '../../dashboard/regionTime.ts';
import { useRegion, useRegionCoverage, useRegionHour } from '../../dashboard/widgetTime.ts';

const NO_LINES: readonly string[] = [];

const rowKey = (row: TemplateRow) => row.key;
const children = (row: TemplateRow) => row.kind === 'obsolete' ? row.children : undefined;
const closedObsolete = () => false;
const TREE: TableTree<TemplateRow> = { children, initiallyOpen: closedObsolete };
const entryOf = (row: TemplateRow) => row.kind === 'template' ? row.entry : null;

export default function DivisionTemplateWidget({ widget, runtime, active, setOption, onDiagnostics, headerSlot }: WidgetProps) {
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const drillTime = drillControl.region.action;
  const drillRegion = useRegion(widget, drillTime);
  const drillCoverage = useRegionCoverage(runtime, drillRegion);
  const drillAt = useRegionHour(runtime, drillRegion, active && drill.breakdown !== null);
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const gate = useGate(runtime, widget, SPEC, active, { country: { stored: widget.options?.country, setOption } });
  const country = gate.country!;
  const hour = gate.hour;
  const tag = gate.open ? country.tag : null;
  const countryLabel = useCountryNameAt(runtime.client, track, tag ?? country.chosen, hour, active);
  const countryName = countryLabel.name;
  const nameDetail = countryLabel.nameFailure ? <span data-detail={countryLabel.nameFailure.message}>{countryName}</span> : undefined;
  const options = resolveOptions(OPTIONS, widget.options);
  const countFor = options.countFor === 'host' ? 'host' : 'owner';
  const state = useHeadProduct(runtime, gate, 'division-templates', tag ? { country: countryFromKey(tag), countFor } : null,
    { active, validate: validateDivisionTemplates, structure: templateStructure });
  useShownEachFrame(runtime.clock, state.shownAt, state.live);
  const data = state.data;
  const shownHour = () => state.at ?? state.hour ?? hour;
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
    const at = shownHour();
    if (!entry || !snapshot || !data || at === null) return;
    setOpened({ template: entry.template, designer: entry.designer ?? 'ordinary', tag: data.tag });
    drillControl.show(entry.template, at);
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
  const tooltip = useCallback((row: TemplateRow) => row.kind === 'obsolete' || !data || shownHour() === null ? { title: `Obsolete templates (${row.kind === 'obsolete' ? row.count : 0})`, rows: [] }
    : fitTooltip([templateTooltip(row.entry, shownHour()!, !!snapshot)], viewportHeight),
  [data, snapshot, viewportHeight, state.hour]);
  const current = data ? data.templates.filter(entry => !entry.obsolete).length : 0;
  const pill = headerSlot && data && createPortal(<span className="pill">{current} {current === 1 ? 'template' : 'templates'}</span>, headerSlot);
  let main;
  if (!track) main = <WidgetState kind="empty" message="Load a campaign to see templates." />;
  else if (!gate.open) main = <GateState gate={gate} loading="Loading templates…" choose="Choose a country to see its templates."
    absent={nameDetail ? 'This country does not exist at this date.' : `${countryName} does not exist at this date.`} detail={nameDetail} />;
  else if (state.error) main = <WidgetState kind="error" failure={state.error} onRetry={state.retry} />;
  else if (!data) main = <WidgetState kind="loading" label="Loading templates…" />;
  else if (!rows.length) main = <WidgetState kind="empty" message={nameDetail ? 'No division templates at this date.' : `${countryName} has no division templates at this date.`} detail={nameDetail} />;
  else main = <Table rowRevision={row => [row, columns]} label="Division templates" rows={rows} columns={columns} rowKey={rowKey} tree={TREE}
    sort={tableSortOf(options.tableSort, DEFAULT_SORT)} setOption={setOption} cellBars={options.cellBars === true}
    rankMarkers={false} density={options.density === 'compact' ? 'compact' : 'comfortable'} narrow={narrowTableOf(options)}
    onRowClick={snapshot ? open : undefined} tooltip={tooltip} hoursPerSecond={hoursPerSecond} />;
  const draw = snapshot;
  const root = useCallback((element: HTMLDivElement | null) => { anchor.current = element; state.root(element); }, [state.root]);
  return <div ref={root} className="division-template" data-detail={countryLabel.nameFailure?.message}>{pill}
    { }
    {country.chosen && country.error && country.historyHeld ? <WidgetState kind="error" failure={country.error} onRetry={country.retry} /> : null}
    <DrillHost drill={drillControl} runtime={runtime} subject={drill.breakdown !== null && !!draw && !!opened} mode={options.drill === 'popout' ? 'popout' : 'inline'}
      title="Division designer" day={drillAt === null ? '' : formatDateTime(drillAt)} canFollow anchor={anchor}
      onClose={() => { dispatch({ type: 'close' }); setOpened(null); setWindowLines(NO_LINES); }} main={main}>
      {draw && opened && track && drillAt === null && <WidgetState kind="outside" read={drillCoverage} />}
      {draw && opened && track && drillAt !== null && draw({ client: runtime.client, source: track.source, branchId: track.branch.id, hour: drillAt,
        tag: opened.tag, template: opened.template, designer: opened.designer, onDiagnostics: onWindowDiagnostics }, active)}
    </DrillHost></div>;
}
