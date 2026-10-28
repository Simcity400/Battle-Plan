import { createPortal } from 'react-dom';
import { useCallback, useMemo, useReducer, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, reduceDrill } from '../../charts/drill/drillState';
import { dayLabel } from '../../charts/draw';
import FlowDiagram from '../../charts/flow/FlowDiagram';
import type { FlowHit } from '../../charts/flow/FlowDiagram';
import type { FlowLink, FlowNode } from '../../charts/flow/layout';
import { jumpItem } from '../../charts/jump';
import { useCountryColours } from '../../charts/race/countryColours';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf } from '../../charts/table/options';
import { useTableHour } from '../../charts/table/useTableHour';
import { fitTooltip } from '../../charts/tooltip';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useContextMenu } from '../../components/ContextMenu';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { useCountryLooks } from '../../dashboard/countryLooks';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { groupReasonAt, preparedAt, tagAt } from '../../dashboard/economyHistory';
import { EQUIPMENT_UNNAMED } from '../../dashboard/EquipmentCell';
import { countryMatch, MEMBERSHIP_PREPARING } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackSpeed } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { ProductionCatalogueReply } from '../../generated/dashboard';
import { COUNTRY, OPTIONS } from './definition';
import { holderLicencesAt, holdersAt, licencePairs, licencesAt, PRODUCTION_CATALOGUE, rosterKnown, validateLicences } from './licencesData';
import type { LicenceAt, LicenceWidth } from './licencesData';
import '../trade-flows/tradeFlows.css';
import './productionLicences.css';

const NO_WARNINGS: readonly string[] = [];
const OTHER = '\u0000other';
const PARAMS_VIEW = 'licences';

/** Who builds whose designs under licence at the moment shown: licensors left,
 * licensees right, a ribbon per pair; a country's flag or name opens its licences. */
export default function ProductionLicencesWidget({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
  const playbackHour = useTableHour(runtime.clock, active && widget.time.mode === 'follow');
  const hoursPerSecond = usePlaybackSpeed(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const width: LicenceWidth = options.width === 'factories' ? 'factories' : 'licences';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = Math.floor(window?.lastHour ?? track?.branch.lastHour ?? 0);
  const anchor = useRef<HTMLDivElement>(null);
  const menu = useContextMenu('Licence actions');
  const [savedDrill, dispatch] = useReducer(reduceDrill, CLOSED);
  // A country is picked as its holders in one recording's branch, so a rename keeps it and another branch drops it.
  const scope = useMemo(() => ({}), [runtime.client, track?.source.id, track?.source.revision, track?.branch.id]);
  const [choice, setChoice] = useState<{ scope: object; tag: string; hour: number; holders: readonly string[] } | null>(null);
  const picked = choice?.scope === scope ? choice : null;
  const drill = picked ? savedDrill : CLOSED;
  const [capacity, setCapacity] = useState(8);
  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const fallback = useMemo(() => selectedTags(widget.countries, track), [widget.countries, track]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? hour),
    [widget.countries, fallback, groups.data, track?.branch.lastHour, hour]);
  const request = track ? { sourceId: track.source.id, revision: track.source.revision, product: PRODUCTION_CATALOGUE,
    params: { branchId: track.branch.id, view: PARAMS_VIEW, tags: [] } } : null;
  const { data, error, retry } = useProduct<ProductionCatalogueReply>(runtime.client, request, active, validateLicences, track?.source);
  useDiagnostics(onDiagnostics, data?.warnings ?? NO_WARNINGS);
  const rows = useMemo(() => data && match.ready && !match.reason ? licencesAt(data, hour, match.matches) : [], [data, match, hour]);
  const pairs = useMemo(() => licencePairs(rows, width).filter(pair => pair.width > 0), [rows, width]);
  const tags = useMemo(() => [...new Set(rows.flatMap(row => [row.licensor, row.licensee]))], [rows]);
  const looks = useCountryLooks(runtime.client, track, tags, hour, active);
  const colour = useCountryColours(tags, looks.color, tokens, looks);
  const { nodes, links } = useMemo(() => {
    const perSide = Math.max(1, capacity);
    const totals = (side: 'licensor' | 'licensee') => {
      const sums = new Map<string, number>();
      for (const pair of pairs) sums.set(pair[side], (sums.get(pair[side]) ?? 0) + pair.width);
      const ranked = [...sums].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
      const kept = new Set(ranked.length > perSide ? ranked.slice(0, perSide - 1).map(([tag]) => tag) : ranked.map(([tag]) => tag));
      return { kept, folded: ranked.length > kept.size };
    };
    const left = totals('licensor'), right = totals('licensee');
    const node = (tag: string, side: 'source' | 'target'): FlowNode => tag === OTHER ? { id: `${side}:${OTHER}`, side, label: 'Other', color: tokens.muted, order: '~', last: true }
      : { id: `${side}:${tag}`, side, label: looks.name(tag, hour), color: colour(tag, hour), flag: looks.flag(tag, hour), order: tag,
        countryFlag: { flag: looks.flag(tag, hour), flagReason: looks.flagReason?.(tag, hour), flagPending: looks.flagPending?.(tag, hour) } };
    const nodeMap = new Map<string, FlowNode>();
    const linkMap = new Map<string, FlowLink>();
    for (const pair of pairs) {
      const source = left.kept.has(pair.licensor) ? pair.licensor : OTHER, target = right.kept.has(pair.licensee) ? pair.licensee : OTHER;
      const from = node(source, 'source'), to = node(target, 'target');
      nodeMap.set(from.id, from); nodeMap.set(to.id, to);
      const id = `${from.id}>${to.id}`;
      const held = linkMap.get(id);
      linkMap.set(id, held ? { ...held, value: held.value + pair.width } : { id, source: from.id, target: to.id, value: pair.width, color: from.color, order: id });
    }
    return { nodes: [...nodeMap.values()], links: [...linkMap.values()] };
  }, [pairs, capacity, looks, colour, tokens.muted, hour]);
  const tooltip = useCallback((hit: FlowHit): TooltipContent | null => {
    const inside = (row: LicenceAt, id: string, side: 'licensor' | 'licensee') => {
      const tag = id.slice(id.indexOf(':') + 1);
      return tag === OTHER ? !nodes.some(node => node.id === `${side === 'licensor' ? 'source' : 'target'}:${row[side]}`) : row[side] === tag;
    };
    const chosen = hit.kind === 'node' ? rows.filter(row => inside(row, hit.node.id, hit.node.side === 'source' ? 'licensor' : 'licensee'))
      : rows.filter(row => inside(row, hit.link.source, 'licensor') && inside(row, hit.link.target, 'licensee'));
    const title = hit.kind === 'node' ? hit.node.label : `${nodes.find(node => node.id === hit.link.source)?.label} → ${nodes.find(node => node.id === hit.link.target)?.label}`;
    return fitTooltip([{ title, subtitle: dayLabel(hour), rows: chosen.flatMap(row => [{ label: row.designs.map(design => design.name ?? EQUIPMENT_UNNAMED).join(', ') || EQUIPMENT_UNNAMED, strong: true },
      { label: `${looks.name(row.licensor, hour)} → ${looks.name(row.licensee, hour)}`, tone: 'muted' as const },
      { label: 'Civilian factories', value: row.lent === null ? '—' : row.lent.toString() },
      ...row.since !== null ? [{ label: 'Since', value: dayLabel(row.since) }] : []]) }], viewport);
  }, [rows, nodes, looks, hour, viewport]);
  const openCountry = (node: FlowNode) => {
    const tag = node.id.slice(node.id.indexOf(':') + 1);
    if (tag === OTHER || !data) return;
    setChoice({ scope, tag, hour, holders: holdersAt(data, tag, hour) });
    dispatch({ type: 'open', breakdown: COUNTRY.id, hour, playing: runtime.clock.getSnapshot().playing, playhead: widget.time.mode === 'follow' ? hour : null });
  };
  const at = drillHour(drill, hour) ?? hour;

  if (!track) return <WidgetState kind="empty" icon="factory" message="Load a campaign to see licences." />;
  if (!window) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  const groupFailure = needsGroups && groups.error ? <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
  if (!data || !match.ready) return <>{groupFailure}<WidgetState kind="loading" label="Loading licences…" /></>;
  if (!preparedAt(data, hour)) return <>{groupFailure}<WidgetState kind="loading" label="This range is still being prepared." /></>;
  // A group whose membership at this hour is unrecorded or empty says so, never "none";
  // membership still being prepared is neither.
  if (!rows.length && !match.known(hour)) return groupFailure ?? <WidgetState kind="loading" label={MEMBERSHIP_PREPARING} />;
  const groupReason = groupReasonAt(widget.countries, groups.data, hour, track.branch.lastHour);
  if (groupReason) return <>{groupFailure}<WidgetState kind={groupReason.kind} message={groupReason.message} /></>;
  if (!rosterKnown(data, hour)) return <>{groupFailure}<WidgetState kind="unavailable" message="Licences were not recorded at this date." /></>;
  const header = headerSlot && createPortal(<span className="pill">{rows.length.toLocaleString()} licences</span>, headerSlot);
  const main = <div className="production-licences">
    <FlowDiagram label="Production licences: licensors to licensees" nodes={nodes} links={links} stretch={width} slots={capacity + 1} hour={hour}
      hoursPerSecond={hoursPerSecond} active={active && !(drill.breakdown && options.drill !== 'popout')} tooltip={tooltip} onCapacity={setCapacity}
      message={rows.length && !pairs.length ? 'No licence lends civilian factories at this date.' : rows.length ? null : 'No production licences at this date.'}
      onLabelPick={openCountry} onPick={hit => { if (hit.kind === 'node') openCountry(hit.node); }}
      onContextMenu={(hit, x, y) => { if (hit?.kind === 'node') menu.open(x, y, [{ label: 'Show licences', run: () => openCountry(hit.node) }]); }} />
    {menu.menu}
  </div>;
  const drillAt = drill.follow ? hour : at;
  // The picked holders' tag at the drill's hour; null before any of them was read with one.
  const drillTag = picked ? picked.holders.map(holder => tagAt(data.holders[holder], drillAt)).find(tag => tag !== null) ?? null : null;
  return <div ref={anchor} className="production-licences-host">{groupFailure}
    {header}
    <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown !== null && picked !== null} anchor={anchor}
      title={picked ? `${drillTag === null ? looks.name(picked.tag, picked.hour) : looks.name(drillTag, drillAt)}: licences` : COUNTRY.label} day={dayLabel(at)} following={drill.follow} canFollow={widget.time.mode === 'follow'}
      onClose={() => dispatch({ type: 'close' })} onFollow={() => dispatch({ type: 'follow' })} main={main}>
      {picked && <CountryLicences data={data} holders={picked.holders} tag={drillTag} hour={drillAt} runtime={runtime} track={track} active={active} narrow={narrowTableOf(options)} />}
    </DrillHost>
  </div>;
}

/** One country's licences granted and held at the drill's hour, by its holders' tags then. */
function CountryLicences({ data, holders, tag, hour, runtime, track, active, narrow }: {
  data: ProductionCatalogueReply; holders: readonly string[]; tag: string | null; hour: number; runtime: WidgetProps['runtime']; track: NonNullable<ReturnType<WidgetProps['runtime']['tracks']['find']>>;
  active: boolean; narrow: ReturnType<typeof narrowTableOf>;
}) {
  const viewport = useViewportHeight();
  const rows = useMemo(() => holderLicencesAt(data, holders, hour), [data, holders, hour]);
  const wanted = useMemo(() => [...new Set(rows.flatMap(row => [row.licensor, row.licensee]))].map(country => ({ hour, tag: country })), [rows, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const columns = useMemo<readonly TableColumn<LicenceAt>[]>(() => [
    { key: 'licensor', header: 'Licensor', label: 'Licensor', width: 'minmax(var(--table-name-min), 1fr)', value: row => row.licensor, render: row => <CountryCell {...labelAt(hour)(row.licensor)} /> },
    { key: 'licensee', header: 'Licensee', label: 'Licensee', width: 'minmax(var(--table-name-min), 1fr)', value: row => row.licensee, render: row => <CountryCell {...labelAt(hour)(row.licensee)} /> },
    { key: 'design', header: 'Design', label: 'Design', width: 'minmax(var(--table-name-min), 1.4fr)', value: row => row.designs[0]?.name ?? null, renderMissing: true,
      text: row => row.designs.map(design => design.name ?? EQUIPMENT_UNNAMED).join(', '), render: row => row.designs.map(design => design.name ?? EQUIPMENT_UNNAMED).join(', ') || EQUIPMENT_UNNAMED },
    { key: 'factories', header: 'Factories', label: 'Civilian factories', numeric: true, width: 'max-content', priority: 2, renderMissing: true,
      value: row => row.lent, render: row => row.lent === null ? '—' : row.lent.toString() },
    { key: 'since', header: 'Since', label: 'Since', numeric: true, width: 'max-content', priority: 1, value: row => row.since,
      text: row => row.since === null ? '—' : dayLabel(row.since), render: row => row.since === null ? '—' : dayLabel(row.since) },
  ], [labelAt, hour]);
  if (tag === null) return <p className="drill-empty">This country was not recorded at this date.</p>;
  if (!rows.length) return <p className="drill-empty">No licences at this date.</p>;
  return <div className="production-licences-table"><Table label={`Licences of ${tag}`} rows={rows} columns={columns} rowKey={row => row.key}
    sort={{ column: 'since', direction: 'descending' }} narrow={narrow}
    tooltip={row => fitTooltip([{ title: row.designs.map(design => design.name ?? EQUIPMENT_UNNAMED).join(', ') || EQUIPMENT_UNNAMED,
      rows: [{ label: 'Civilian factories lent', value: row.lent === null ? '—' : row.lent.toString() }, { label: 'Required', value: row.required === null ? '—' : row.required.toString() },
        ...row.since !== null ? [{ label: 'Since', value: dayLabel(row.since) }] : []] }], viewport)}
    menu={row => row.since === null ? [] : [jumpItem(runtime, track.id, row.since)]} /></div>;
}
