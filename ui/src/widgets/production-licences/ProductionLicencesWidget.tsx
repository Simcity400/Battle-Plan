import { createPortal } from 'react-dom';
import { useCallback, useMemo,  useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { dayLabel } from '../../charts/draw';
import FlowDiagram from '../../charts/flow/FlowDiagram';
import type { FlowHit } from '../../charts/flow/FlowDiagram';
import type { FlowLink, FlowNode } from '../../charts/flow/layout';
import { jumpItem } from '../../charts/jump';
import { useCountryColours } from '../../charts/race/countryColours';
import Table, { CountryCell } from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { narrowTableOf } from '../../charts/table/options';
import { countryNameDetails, fitTooltip } from '../../charts/tooltip.ts';
import type { TooltipContent } from '../../charts/tooltip';
import { useChartTokens, useViewportHeight } from '../../charts/useChartTokens';
import { useContextMenu } from '../../components/ContextMenu';
import { useAtHead, useIdentityHour } from '../../dashboard/atHead';
import { trackHourOf } from '../../dashboard/settledHour';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { lookHour, looksRead, useCountryLooks } from '../../dashboard/countryLooks';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { useHeadProduct } from '../../dashboard/headProduct';
import { useGate } from '../../dashboard/regionGate';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackMotion } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownEachFrame } from '../../dashboard/widgetPerf.ts';
import { COUNTRY, OPTIONS } from './definition';
import { countryLicences, groupReasonAt, licencePairs, licenceRevision, licencesAt, pickedTag, PRODUCTION_LICENCES, slotOf, validateLicences } from './licencesData';
import type { LicenceRow, LicenceWidth, PickedCountry } from './licencesData';
import './productionLicences.css';
import { useRegion } from '../../dashboard/widgetTime.ts';
import { productRequest } from '../../dashboard/request.ts';

const NO_WARNINGS: readonly string[] = [];
const NO_LICENCES: ReturnType<typeof licencesAt> = { rows: [], unattributed: 0 };
const OTHER = '\u0000other';
const UNNAMED = '\u0000unnamed';
const UNNAMED_COUNTRY = 'Country name unavailable';
const DESIGN_UNNAMED = 'Design name unavailable';
const PARAMS = {};
const nodeTag = (tag: string | null) => tag ?? UNNAMED;
const designsText = (row: LicenceRow) => row.designs.map(design => design.name ?? DESIGN_UNNAMED).join(', ') || DESIGN_UNNAMED;

export default function ProductionLicencesWidget({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
  const hoursPerSecond = usePlaybackMotion(runtime.clock);
  const tokens = useChartTokens();
  const viewport = useViewportHeight();
  const options = resolveOptions(OPTIONS, widget.options);
  const width: LicenceWidth = options.width === 'factories' ? 'factories' : 'licences';
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const head = useHeadProduct(runtime, useGate(runtime, widget), PRODUCTION_LICENCES, track ? PARAMS : null, { active, validate: validateLicences });
  const following = active && widget.time.mode === 'follow';
  const stepped = useAtHead(runtime.clock, following, clockHour => {
    const at = track ? trackHourOf(widget, track, runtime.tracks, clockHour) : null;
    return at === null || !track ? null : lookHour(track.branch, at)?.hour ?? null;
  });
  const hour = head.hour ?? stepped ?? track?.branch.lastHour ?? 0;
  useShownEachFrame(runtime.clock, head.shownAt, head.live);
  const anchor = useRef<HTMLDivElement>(null);
  const menu = useContextMenu('Licence actions');
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const [picked, setPicked] = useState<PickedCountry | null>(null);
  const [capacity, setCapacity] = useState(8);
  const needsGroups = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && needsGroups);
  const fallback = useMemo(() => selectedCountryKeys(widget.countries, track), [widget.countries, track]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? hour), [widget.countries, fallback, groups.data, track?.branch.lastHour, hour]);
  const data = head.data;
  const listed = useMemo(() => data && match.ready && !match.reason ? licencesAt(data, hour, match.matches) : NO_LICENCES, [data, match, hour]);
  const rows = listed.rows;
  const warnings = useMemo(() => listed.unattributed ? [...data?.warnings ?? [], `${listed.unattributed} licences whose countries were not recorded at this date may also match`]
    : data?.warnings ?? NO_WARNINGS, [listed.unattributed, data]);
  useDiagnostics(onDiagnostics, warnings);
  const pairs = useMemo(() => licencePairs(rows, width).filter(pair => pair.width > 0), [rows, width]);
  const tags = useMemo(() => [...new Set(rows.flatMap(row => [row.licensor, row.licensee]).filter((tag): tag is string => tag !== null))], [rows]);
  const identityHour = useIdentityHour(runtime.client, track, tags, runtime.clock, following && head.hour === null,
    clockHour => track ? trackHourOf(widget, track, runtime.tracks, clockHour) : null);
  const lookRead = looksRead(following && head.hour === null, identityHour, hour);
  const looks = useCountryLooks(runtime.client, track, tags, lookRead.hour, active, undefined, undefined, lookRead.exact);
  const colour = useCountryColours(tags, looks.color, tokens, looks);
  const partyName = useCallback((tag: string | null) => tag === null ? UNNAMED_COUNTRY : looks.name(tag, hour), [looks, hour]);
  const { nodes, links } = useMemo(() => {
    const perSide = Math.max(1, capacity);
    const totals = (side: 'licensor' | 'licensee') => {
      const sums = new Map<string, number>();
      for (const pair of pairs) sums.set(nodeTag(pair[side]), (sums.get(nodeTag(pair[side])) ?? 0) + pair.width);
      const ranked = [...sums].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
      return new Set(ranked.length > perSide ? ranked.slice(0, perSide - 1).map(([tag]) => tag) : ranked.map(([tag]) => tag));
    };
    const left = totals('licensor'), right = totals('licensee');
    const node = (tag: string, side: 'source' | 'target'): FlowNode => tag === OTHER ? { id: `${side}:${OTHER}`, side, label: 'Other', color: tokens.muted, order: '~', last: true }
      : tag === UNNAMED ? { id: `${side}:${UNNAMED}`, side, label: UNNAMED_COUNTRY, color: tokens.muted, order: '~~', last: true }
        : { id: `${side}:${tag}`, side, label: looks.name(tag, hour), color: colour(tag, hour), flag: looks.flag(tag, hour), order: tag,
          countryFlag: { nameFailure: looks.nameFailure?.(tag, hour), flagDetail: looks.flagDetail?.(tag, hour), flag: looks.flag(tag, hour), flagReason: looks.flagReason?.(tag, hour), flagPending: looks.flagPending?.(tag, hour) } };
    const nodeMap = new Map<string, FlowNode>();
    const linkMap = new Map<string, FlowLink>();
    for (const pair of pairs) {
      const from = node(left.has(nodeTag(pair.licensor)) ? nodeTag(pair.licensor) : OTHER, 'source');
      const to = node(right.has(nodeTag(pair.licensee)) ? nodeTag(pair.licensee) : OTHER, 'target');
      nodeMap.set(from.id, from);
      nodeMap.set(to.id, to);
      const id = `${from.id}>${to.id}`;
      const held = linkMap.get(id);
      linkMap.set(id, held ? { ...held, value: held.value + pair.width } : { id, source: from.id, target: to.id, value: pair.width, color: from.color, order: id });
    }
    return { nodes: [...nodeMap.values()], links: [...linkMap.values()] };
  }, [pairs, capacity, looks, colour, tokens.muted, hour]);
  const tooltip = useCallback((hit: FlowHit): TooltipContent | null => {
    const inside = (row: LicenceRow, id: string, side: 'licensor' | 'licensee') => {
      const tag = id.slice(id.indexOf(':') + 1);
      return tag === OTHER ? !nodes.some(node => node.id === `${side === 'licensor' ? 'source' : 'target'}:${nodeTag(row[side])}`) : nodeTag(row[side]) === tag;
    };
    const chosen = hit.kind === 'node' ? rows.filter(row => inside(row, hit.node.id, hit.node.side === 'source' ? 'licensor' : 'licensee'))
      : rows.filter(row => inside(row, hit.link.source, 'licensor') && inside(row, hit.link.target, 'licensee'));
    const title = hit.kind === 'node' ? hit.node.label : `${nodes.find(node => node.id === hit.link.source)?.label} → ${nodes.find(node => node.id === hit.link.target)?.label}`;
    return fitTooltip([{ title, ...countryNameDetails(...chosen.flatMap(row => [row.licensor, row.licensee].map(tag => ({ nameFailure: tag === null ? undefined : looks.nameFailure?.(tag, hour) })))),
      subtitle: dayLabel(hour), rows: chosen.flatMap(row => [{ label: designsText(row), strong: true },
      { label: `${partyName(row.licensor)} → ${partyName(row.licensee)}`, tone: 'muted' as const },
      { label: 'Civilian factories', value: row.lent.toString() }, { label: 'Since', value: dayLabel(row.since) }]) }], viewport);
  }, [rows, nodes, partyName, looks, hour, viewport]);
  const openCountry = (node: FlowNode) => {
    const tag = node.id.slice(node.id.indexOf(':') + 1);
    if (tag === OTHER || tag === UNNAMED || !data) return;
    setPicked({ slot: slotOf(data, tag), tag });
    dispatch({ type: 'open', breakdown: COUNTRY.id, hour, follow: drillControl.followsAt(hour) });
  };
  const drillHour = drillControl.hour;
  const drillAt = drillHour ?? hour;
  const sharesParent = drillControl.following && widget.time.mode === 'follow';
  const ownDay = picked !== null && drill.breakdown !== null && !sharesParent && track && drillHour !== null ? productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: PRODUCTION_LICENCES, params: { branchId: track.branch.id, hour: drillHour } }) : null;
  const own = useProduct(runtime.client, ownDay, active, validateLicences, track?.source);
  const drillData = ownDay ? own.data : data;
  const drillTag = drillData && picked ? pickedTag(drillData, picked) : null;
  const titleRead = picked ? { tag: drillTag ?? picked.tag, hour: drillAt } : null;
  const titleNames = useCountryLabelsAt(runtime.client, track, titleRead ? [titleRead] : [], active);

  if (!track) return <WidgetState kind="empty" icon="factory" message="Load a campaign to see licences." />;
  const groupFailure = needsGroups && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  const groupReason = data && match.ready ? groupReasonAt(widget.countries, groups.data, hour, track.branch.lastHour) : null;
  const parentState = head.read === 'off' ? <WidgetState kind="outside" />
    : head.error ? <WidgetState kind="error" failure={head.error} onRetry={head.retry} />
      : groupFailure && !groups.data ? null
        : match.reason ? <WidgetState kind="unavailable" message={match.reason} />
          : !data || !match.ready ? <div ref={head.root}><WidgetState kind="loading" label="Loading licences…" /></div>
            : groupReason ? <div ref={head.root}><WidgetState kind={groupReason.kind} message={groupReason.message} /></div>
              : data.missing ? <div ref={head.root}><WidgetState kind="unavailable" message={data.missing} /></div> : undefined;
  const header = parentState === undefined && headerSlot && createPortal(<span className="pill">{rows.length.toLocaleString()}{listed.unattributed ? '+' : ''} licences</span>, headerSlot);
  const main = parentState !== undefined ? parentState : <div ref={head.root} className="production-licences">
    <FlowDiagram label="Production licences: licensors to licensees" nodes={nodes} links={links} stretch={width} slots={capacity + 1} hour={hour}
      hoursPerSecond={hoursPerSecond} active={active && !(drill.breakdown && options.drill !== 'popout')} tooltip={tooltip} onCapacity={setCapacity}
      message={rows.length && !pairs.length ? 'No licence lends civilian factories at this date.' : rows.length ? null
        : listed.unattributed ? "Some licences' countries were not recorded at this date." : 'No production licences at this date.'}
      onLabelPick={openCountry} onPick={hit => { if (hit.kind === 'node') openCountry(hit.node); }}
      onContextMenu={(hit, x, y) => { if (hit?.kind === 'node') menu.open(x, y, [{ label: 'Show licences', run: () => openCountry(hit.node) }]); }} />
    {menu.menu}
  </div>;
  return <div ref={anchor} className="production-licences-host">{groupFailure}
    {header}
    <DrillHost drill={drillControl} runtime={runtime} subject={drill.breakdown !== null && picked !== null} mode={options.drill === 'popout' ? 'popout' : 'inline'} anchor={anchor}
      title={titleRead ? `${titleNames.labelAt(titleRead.hour)(titleRead.tag).name}: licences` : COUNTRY.label} day={dayLabel(drillAt)}
      canFollow onClose={() => { dispatch({ type: 'close' }); setPicked(null); }} main={main}>
      {picked && (own.error && ownDay ? <WidgetState kind="error" failure={own.error} onRetry={own.retry} />
        : !drillData ? <WidgetState kind="loading" label="Loading licences…" />
          : drillData.missing ? <p className="drill-empty">{drillData.missing}</p>
            : <CountryLicences data={drillData} picked={picked} tag={drillTag} hour={drillAt} runtime={runtime} track={track} active={active} narrow={narrowTableOf(options)} />)}
    </DrillHost>
  </div>;
}

function CountryLicences({ data, picked, tag, hour, runtime, track, active, narrow }: {
  data: NonNullable<ReturnType<typeof validateLicences>>; picked: PickedCountry; tag: string | null; hour: number; runtime: WidgetProps['runtime'];
  track: NonNullable<ReturnType<WidgetProps['runtime']['tracks']['find']>>; active: boolean; narrow: ReturnType<typeof narrowTableOf>;
}) {
  const viewport = useViewportHeight();
  const held = useMemo(() => countryLicences(data, picked), [data, picked]);
  const rows = held.rows;
  const wanted = useMemo(() => [...new Set(rows.flatMap(row => [row.licensor, row.licensee]).filter((party): party is string => party !== null))].map(party => ({ hour, tag: party })), [rows, hour]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const columns = useMemo<readonly TableColumn<LicenceRow>[]>(() => [
    { key: 'licensor', header: 'Licensor', label: 'Licensor', width: 'minmax(var(--table-name-min), 1fr)', value: row => row.licensor, renderMissing: true,
      render: row => row.licensor === null ? UNNAMED_COUNTRY : <CountryCell {...labelAt(hour)(row.licensor)} /> },
    { key: 'licensee', header: 'Licensee', label: 'Licensee', width: 'minmax(var(--table-name-min), 1fr)', value: row => row.licensee, renderMissing: true,
      render: row => row.licensee === null ? UNNAMED_COUNTRY : <CountryCell {...labelAt(hour)(row.licensee)} /> },
    { key: 'design', header: 'Design', label: 'Design', width: 'minmax(var(--table-name-min), 1.4fr)', value: row => row.designs[0]?.name ?? null, renderMissing: true,
      text: designsText, render: designsText },
    { key: 'factories', header: 'Factories', label: 'Civilian factories', numeric: true, width: 'max-content', priority: 2, value: row => row.lent, render: row => row.lent.toString() },
    { key: 'since', header: 'Since', label: 'Since', numeric: true, width: 'max-content', priority: 1, value: row => row.since, text: row => dayLabel(row.since), render: row => dayLabel(row.since) },
  ], [labelAt, hour]);
  if (!rows.length) return <p className="drill-empty">{held.unattributed ? "Some licences' licensors were not recorded at this date." : 'No licences at this date.'}</p>;
  return <div className="production-licences-table"><Table rowRevision={row => [...licenceRevision(row), row.licensor === null ? null : labelAt(hour)(row.licensor),
    row.licensee === null ? null : labelAt(hour)(row.licensee)]} label={`Licences of ${tag ?? picked.tag}`} rows={rows} columns={columns} rowKey={row => row.key}
    sort={{ column: 'since', direction: 'descending' }} narrow={narrow}
    tooltip={row => fitTooltip([{ title: designsText(row), rows: [{ label: 'Civilian factories lent', value: row.lent.toString() }, { label: 'Required', value: row.required.toString() },
      { label: 'Since', value: dayLabel(row.since) }] }], viewport)}
    menu={row => [jumpItem(runtime, track.id, row.since)]} /></div>;
}
