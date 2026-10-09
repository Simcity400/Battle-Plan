import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import FlowDiagram from '../../charts/flow/FlowDiagram';
import DrillHost from '../../charts/drill/DrillHost';
import { CLOSED, drillHour, pinnedBreakdown, reduceDrill } from '../../charts/drill/drillState';
import type { FlowHit, FlowLink } from '../../charts/flow/FlowDiagram';
import { countryFlowColours, countryFlowNodes, flowOtherColour } from '../../charts/flow/countries';
import type { SummedNode } from '../../charts/flow/sum';
import { jumpItem } from '../../charts/jump';
import { useChartTokens } from '../../charts/useChartTokens';
import { formatDateTime } from '../../dashboard/calendar';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { nameKey, useCountryFactsRead } from '../../dashboard/countryNamesAt';
import { STYLES } from '../../dashboard/useCountriesAt';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { peaceCaptureLabel, peaceDealAt, peaceDealHour, peaceDealLabel, peaceObservationLabel } from '../../dashboard/peaceDealsData';
import { usePlaybackHour } from '../../dashboard/playback';
import { usePeaceDeals } from '../../dashboard/usePeaceDeals';
import { useStateNamesAt } from '../../dashboard/useStateNamesAt';
import { OPTIONS, TRANSFER_BREAKDOWN } from './definition';
import TransferDrill from './TransferDrill';
import { transferDrillKey, transferDrillOpen, transferDrawnSelection, transferHitSelection, transferRestoreFocus, transferScopeLinks, transferSelected, transferSelection, transferSelectionToken, transferTime } from './transferDrillData';
import { buildTransferDiagram, chosenConference, conferenceChoiceId, conferenceToken, decodeTransfers, transferColourPlan, transferCutoff, transferDiagramDescription, transferGapCount, transferGapWindow,
  transfersThrough, transferNodeCountries, transferLookPlan, victoryPointNote, visibleConferences } from './territoryData';
import type { TransferDrawnSelection } from './transferDrillData';
import type { TransferLink, TransferMeasure } from './territoryData';
import { transferNodeTooltip, transferRibbonTooltip } from './territoryTooltip';
import type { TransferTipRequest } from './territoryTooltip';
import './territoryTransfers.css';

const NO_WARNINGS: string[] = [];
const NO_REQUESTS: TransferTipRequest = { countries: [], states: [] };

/** A two-column peace transfer graph, shaped by the same kit as Trade flows. */
export default function TerritoryTransfersWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const [drill, dispatchDrill] = useReducer(reduceDrill, CLOSED);
  const [drillIds, setDrillIds] = useState<readonly string[]>([]);
  const [drillConference, setDrillConference] = useState<string | null>(null);
  const drillAnchor = useRef<HTMLDivElement>(null);
  const drillBody = useRef<HTMLDivElement>(null);
  const following = widget.time.mode === 'follow';
  usePlaybackHour(runtime.clock, active && (following || drill.breakdown != null && drill.follow));
  const { track, data, error, retry } = usePeaceDeals(widget, runtime, active);
  const tokens = useChartTokens();
  const options = resolveOptions(OPTIONS, widget.options);
  const measure = options.measure as TransferMeasure;
  const scope = options.scope as string;
  const selectedToken = String(options.conference ?? '');
  const selectedId = track ? conferenceChoiceId(selectedToken, track.source.metadata.campaignKey, track.branch.id) : null;
  const showParts = options.show === 'parts';
  const storedSelection = useMemo(() => track ? transferSelection(String(options.selection ?? ''), track.source.metadata.campaignKey, track.branch.id) : null,
    [options.selection, track]);
  const time = track ? transferTime(widget, track, runtime.tracks, runtime.clock.getHour(), showParts ? storedSelection?.fromOffset : null) : { moment: -Infinity, from: -Infinity };
  const moment = time.moment;
  const rangeFrom = time.from;
  const transfers = useMemo(() => data && decodeTransfers(data), [data]);
  const conferenceThrough = transfers ? transferCutoff(transfers.conferenceHours, moment) : -Infinity;
  const available = useMemo(() => data ? visibleConferences(data.deals, conferenceThrough, rangeFrom) : [], [data, conferenceThrough, rangeFrom]);
  const conference = useMemo(() => {
    const selected = scope === 'conference' && selectedId != null ? chosenConference(available, selectedId) : null;
    return selected ? peaceDealAt(selected, moment) : null;
  }, [scope, available, selectedId, moment]);
  const scopeLinks = transfers ? transferScopeLinks(transfers, scope === 'conference' ? conference?.id ?? '' : null) : [];
  const through = transfers ? transferCutoff(transfers.hours, moment) : -Infinity;
  const victoryPointsThrough = widget.time.mode !== 'campaign' && transfers ? transferCutoff(transfers.victoryPointHours, moment) : Infinity;
  const shown = useMemo(() => transfersThrough(scopeLinks, through, victoryPointsThrough, rangeFrom), [scopeLinks, through, victoryPointsThrough, rangeFrom]);
  const [capacity, setCapacity] = useState<number | null>(null);
  const diagram = useMemo(() => buildTransferDiagram(shown, measure, capacity), [shown, measure, capacity]);
  const menu = useContextMenu('Territory transfer actions');

  useEffect(() => {
    onDiagnostics(data?.warnings ?? NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  const nodeCountries = useMemo(() => transferNodeCountries(shown), [shown]);
  const fixed = tokens.countryColours === 'fixed';
  const lookPlan = useMemo(() => transferLookPlan(new Map(diagram.nodes.flatMap(node => node.tag != null && nodeCountries.has(node.tag)
    ? [[node.tag, nodeCountries.get(node.tag)!] as const] : [])), track?.branch), [diagram, nodeCountries, track?.branch]);
  const colourMoment = drill.breakdown ? Math.max(moment, drillHour(drill, track ? Math.floor(runtime.clock.getHour() + track.axisOffset) : null) ?? moment) : moment;
  const stylePlan = useMemo(() => transferColourPlan(transfers, track?.branch, colourMoment, true), [transfers, track?.branch, colourMoment]);
  const styles = useCountryFactsRead(runtime.client, track, stylePlan.wanted, active, STYLES, stylePlan.maxHours);
  const nodeLabels = useCountryLabelsAt(runtime.client, track, lookPlan.wanted, active, lookPlan.maxHours);
  const countryColours = useMemo(() => {
    const variants = new Map<string, [number, number, number]>();
    const samples = new Map<string, string>();
    for (const sample of stylePlan.samples) {
      const style = styles.get(nameKey(sample.hour, sample.tag));
      if (!style || typeof style !== 'object' || style.color == null) continue;
      // Identical looks of one holder share one adjustment across dates/scopes.
      const key = JSON.stringify([sample.key, style.color]);
      variants.set(key, style.color);
      samples.set(nameKey(sample.hour, sample.tag), key);
    }
    const colours = countryFlowColours([...variants.keys()], key => variants.get(key) ?? null, tokens.surface, fixed);
    return new Map([...samples].map(([key, variant]) => [key, colours.get(variant)!]));
  }, [stylePlan, styles, tokens.surface, fixed]);
  const nodeColours = useMemo(() => new Map([...nodeCountries].map(([key, country]) => [key, countryColours.get(nameKey(country.hour, country.tag)) ?? flowOtherColour(tokens)])), [nodeCountries, countryColours, tokens]);
  const look = useCallback((key: string) => {
    const sample = nodeCountries.get(key);
    return sample ? nodeLabels.labelAt(sample.hour)(sample.tag) : { name: 'Country identity not recorded', flagPending: false };
  }, [nodeCountries, nodeLabels]);
  const nodeName = useCallback((node: SummedNode) => node.tag != null ? look(node.tag).name
    : `${node.members.length.toLocaleString('en-US')} other ${node.side === 'source' ? 'givers' : 'takers'}`, [look]);
  const otherColour = flowOtherColour(tokens);
  const { nodes, links } = useMemo(() => {
    const countryNodes = countryFlowNodes(diagram.nodes, look, tokens, nodeName,
      node => String(transfers?.countryOrder.get(node.tag!) ?? 0).padStart(10, '0'), nodeColours);
    const idsByNode = new Map<string, string[]>();
    for (const link of diagram.links) for (const node of [link.source, link.target]) {
      const ids = idsByNode.get(node) ?? [];
      for (const part of link.parts) ids.push(part.id);
      idsByNode.set(node, ids);
    }
    const ordinaryNodes = new Set(diagram.nodes.filter(node => node.tag != null).map(node => node.id));
    const links: (FlowLink & TransferDrawnSelection)[] = diagram.links.map(link => {
      const latest = link.parts.flatMap(part => part.states.slice(-1)).reduce((a, b) => a.dates.afterHour >= b.dates.afterHour ? a : b);
      const country = options.colour === 'taker' ? latest.toCountry : latest.fromCountry;
      const node = options.colour === 'taker' ? link.target : link.source;
      return { id: link.id, source: link.source, target: link.target, value: link.value,
        ...transferDrawnSelection(link.parts.map(part => part.id), scope === 'conference' ? conference?.id ?? null : null), color: !ordinaryNodes.has(node) ? otherColour : countryColours.get(nameKey(country.hour, country.tag)) ?? otherColour, order: link.id };
    });
    return { nodes: countryNodes.nodes.map(node => ({ ...node, ...transferDrawnSelection([...new Set(idsByNode.get(node.id) ?? [])], scope === 'conference' ? conference?.id ?? null : null) })), links };
  }, [diagram, look, tokens, nodeName, countryColours, options.colour, otherColour, transfers, scope, conference?.id, nodeColours]);
  const nodeById = useMemo(() => new Map(diagram.nodes.map(node => [node.id, node])), [diagram]);
  const linkById = useMemo(() => new Map(diagram.links.map(link => [link.id, link])), [diagram]);
  const linksByNode = useMemo(() => {
    const index = new Map<string, typeof diagram.links>();
    for (const link of diagram.links) for (const id of [link.source, link.target]) {
      const own = index.get(id) ?? [];
      own.push(link);
      index.set(id, own);
    }
    return index;
  }, [diagram]);

  // A hover can contain several conference hours. The shared fact stores keep
  // those separate and request every occurrence in the current tooltip.
  const [requested, setRequested] = useState<TransferTipRequest>(NO_REQUESTS);
  const requestKey = useRef('');
  const ask = useCallback((next: TransferTipRequest) => {
    const key = JSON.stringify(next);
    if (key === requestKey.current) return;
    requestKey.current = key;
    queueMicrotask(() => setRequested(next));
  }, []);
  useEffect(() => { if (!active) { requestKey.current = ''; setRequested(NO_REQUESTS); } }, [active]);
  const hover = useCountryLabelsAt(runtime.client, track, requested.countries, active, Infinity);
  const states = useStateNamesAt(runtime.client, track, requested.states, active);
  const stateNameAt = useCallback((hour: number, state: number) => states(hour, state).name ?? `State ${state} · name not recorded`, [states]);
  const stateMissingAt = useCallback((hour: number, state: number) => states(hour, state).missing, [states]);

  const [gapFrom, until] = useMemo(() => transferGapWindow(conference, shown, track?.branch.firstHour ?? 0, moment),
    [conference, shown, track?.branch.firstHour, moment]);
  const from = Math.max(rangeFrom, gapFrom);
  const gaps = useMemo(() => data ? transferGapCount(data, from, until) : 0, [data, from, until]);
  const gapNote = gaps ? `${gaps.toLocaleString('en-US')} ${gaps === 1 ? 'span was' : 'spans were'} not recorded; totals include only recorded transfer outcomes.` : null;
  const vpNote = victoryPointNote(diagram);
  const captureNote = conference ? peaceCaptureLabel(conference) : null;
  const observationNote = conference ? peaceObservationLabel(conference) : null;
  const note = [gapNote, vpNote, captureNote].filter(Boolean).join(' ') || undefined;
  const tooltipNote = [observationNote, note].filter(Boolean).join(' ') || undefined;
  const tooltip = useCallback((hit: FlowHit) => {
    const tooltipOptions = { measure, note: tooltipNote, nodeName, labelAt: hover.labelAt, stateNameAt, stateMissingAt, ask };
    if (hit.kind === 'link') {
      const link = linkById.get(hit.link.id);
      return link ? transferRibbonTooltip(link, nodeById.get(link.source)!, nodeById.get(link.target)!, tooltipOptions) : null;
    }
    const node = nodeById.get(hit.node.id);
    return node ? transferNodeTooltip(node, linksByNode.get(node.id) ?? [], tooltipOptions) : null;
  }, [measure, tooltipNote, nodeName, hover.labelAt, stateNameAt, stateMissingAt, ask, nodeById, linkById, linksByNode]);

  const onContextMenu = useCallback((hit: FlowHit | null, x: number, y: number) => {
    if (!track) return;
    const parts = hit?.kind === 'link' ? linkById.get(hit.link.id)?.parts ?? []
      : hit?.kind === 'node' ? (linksByNode.get(hit.node.id) ?? []).flatMap(link => link.parts) : shown;
    let first = Infinity;
    let last = -Infinity;
    for (const part of parts) for (const state of part.states) { first = Math.min(first, state.dates.afterHour); last = Math.max(last, state.dates.afterHour); }
    const hours = Number.isFinite(last) ? [...new Set([last, first])] : [conference ? peaceDealHour(conference) : moment];
    menu.open(x, y, hours.map(hour => jumpItem(runtime, track.id, hour)));
  }, [track, runtime, linkById, linksByNode, shown, conference, moment, menu]);

  const playhead = track ? Math.floor(runtime.clock.getHour() + track.axisOffset) : null;
  const selectedIds = showParts ? storedSelection?.ids ?? null : drillIds;
  const missingSelection = useMemo(() => {
    if (!showParts || !selectedIds?.length) return 0;
    const available = new Set(scopeLinks.map(link => link.id));
    return selectedIds.reduce((count, id) => count + Number(!available.has(id)), 0);
  }, [showParts, selectedIds, scopeLinks]);
  const selectedConference = !showParts && drill.breakdown ? drillConference : scope === 'conference' ? conference?.id ?? null : null;
  const cohortKey = useMemo(() => transferDrillKey(track?.source.id ?? '', track?.branch.id ?? '', selectedConference, selectedIds ?? []),
    [track?.source.id, track?.branch.id, selectedConference, selectedIds]);
  const drillSourceLinks = !showParts && drill.breakdown && transfers ? transferScopeLinks(transfers, drillConference) : scopeLinks;
  const drillLinks = useMemo(() => transferSelected(drillSourceLinks, selectedIds ?? []), [drillSourceLinks, selectedIds]);
  const drillAt = drill.breakdown ? drillHour(drill, playhead) ?? moment : moment;
  const drillCutoff = drill.breakdown && drill.follow ? playhead ?? moment : moment;
  const openDrill = useCallback((ids: readonly string[], hour: number, conferenceId: string | null) => {
    setDrillIds([...new Set(ids)]);
    setDrillConference(conferenceId);
    dispatchDrill(transferDrillOpen(hour, following, playhead, runtime.clock.getSnapshot().playing));
  }, [runtime.clock, following, playhead]);
  const onPick = useCallback((hit: FlowHit, hour: number) => {
    const drawn = transferHitSelection(hit);
    if (drawn?.transferIds.length) openDrill(drawn.transferIds, hour, drawn.transferConference);
  }, [openDrill]);
  useEffect(() => { dispatchDrill({ type: 'close' }); setDrillIds([]); setDrillConference(null); }, [track?.source.id, track?.branch.id, scope, selectedToken]);
  const pickDay = useCallback((hour: number) => {
    if (!drill.breakdown) dispatchDrill({ type: 'open', breakdown: 'parts', hour, playing: false, playhead: null });
    else dispatchDrill({ type: 'pick', hour });
  }, [drill.breakdown]);
  const closeDrill = useCallback(() => {
    const closingPanel = drillBody.current?.closest('.drill-popout') ?? null;
    dispatchDrill({ type: 'close' });
    requestAnimationFrame(() => {
      const focused = document.activeElement;
      if (!transferRestoreFocus(focused, document.body, drillAnchor.current, closingPanel)) return;
      drillAnchor.current?.querySelector<HTMLCanvasElement>('.flow-diagram canvas')?.focus({ preventScroll: true });
    });
  }, []);
  const colorOf = useCallback((link: TransferLink) => {
    const state = link.states.at(-1)!;
    const country = options.colour === 'taker' ? state.toCountry : state.fromCountry;
    return countryColours.get(nameKey(country.hour, country.tag)) ?? otherColour;
  }, [countryColours, otherColour, options.colour]);
  const description = useMemo(() => transferDiagramDescription(diagram, measure, nodeName, note), [diagram, nodeName, measure, note]);

  if (!track) return <WidgetState kind="empty" icon="flows" message="Load a campaign to see territory transfers." />;
  if (moment < track.branch.firstHour || moment > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the campaign’s recorded branch." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data || !transfers) return <WidgetState kind="loading" label="Loading territory transfers…" />;
  if (showParts && missingSelection) return <WidgetState kind="unavailable" message={`${missingSelection.toLocaleString('en-US')} selected transfer links are not recorded in this scope.`} />;
  if (scope === 'conference' && !conference) return <WidgetState kind="empty" message={selectedToken ? 'This conference is outside the selected recorded interval.' : 'No peace conferences yet.'} />;
  const message = capacity == null || diagram.links.length ? null : measure === 'victoryPoints' && diagram.missingVictoryPoints ? 'The links’ victory points were not recorded.'
    : shown.length && measure === 'victoryPoints' ? 'The recorded victory points have no positive ribbon width.'
      : gaps || captureNote || conference?.limitations.length ? 'No territory transfers could be attributed in the recorded data.' : 'No peace territory transfers recorded yet.';
  const caption = conference ? peaceDealLabel(conference) : `${widget.time.mode === 'range' ? `Range from ${formatDateTime(rangeFrom)}` : 'Campaign'}${widget.time.mode !== 'campaign' ? ` · up to ${formatDateTime(moment)}` : ''}`;
  const main = <div className="territory-transfers">
    <p className="territory-transfer-caption">{caption} · {measure === 'states' ? 'State count' : 'Victory points'}
      {diagram.nodes.some(node => node.tag == null) ? ` · ${capacity} nodes a side fit this size; the remaining countries are summed` : ''}{note ? ` · ${note}` : ''}</p>
    <p className="chart-line-key">Ribbons in each {options.colour === 'taker' ? 'taker' : 'giver'} country’s map colour</p>
    <button type="button" className="button small transfer-all-parts" onClick={() => openDrill([], moment, scope === 'conference' ? conference?.id ?? null : null)}>All transfer details</button>
    <FlowDiagram label="Territory transfers: losers to winners" nodes={nodes} links={links} stretch={JSON.stringify([track.source.id, track.branch.id, scope, conference?.id ?? null, measure])}
      scale={null} slots={capacity ?? undefined} playbackHour={moment} hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} message={message}
      hour={moment} onPick={onPick} tooltip={tooltip} onContextMenu={onContextMenu} description={description} onCapacity={setCapacity} active={active && (drill.breakdown == null || options.drill === 'popout')} />
  </div>;
  const first = Math.max(track.branch.firstHour, rangeFrom);
  const last = widget.time.mode === 'range' ? moment : track.branch.lastHour;
  const breakdown = <div className="transfer-drill-owner" ref={drillBody}><TransferDrill key={cohortKey} links={drillLinks} measure={measure} day={drillAt} cutoff={drillCutoff} first={first} last={last} from={rangeFrom}
    runtime={runtime} track={track} active={active} colorOf={colorOf} onPick={pickDay}
    onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} /></div>;
  const pin = () => {
    const pinned = pinnedBreakdown(widget, TRANSFER_BREAKDOWN, track.id);
    const offset = drillAt - track.branch.firstHour;
    runtime.addWidgetBelow(widget.id, { ...pinned, time: { mode: 'pin', windows: { [track.id]: { firstOffset: offset, lastOffset: offset } } },
      options: { ...pinned.options, scope: selectedConference == null ? 'campaign' : 'conference', conference: selectedConference == null ? '' : conferenceToken(track.source.metadata.campaignKey, track.branch.id, selectedConference), selection: transferSelectionToken(track.source.metadata.campaignKey, track.branch.id, selectedIds ?? [], Number.isFinite(rangeFrom) ? rangeFrom - track.branch.firstHour : null) } });
  };
  return <div className="territory-transfer-host" ref={drillAnchor}>
    {showParts ? storedSelection == null ? <WidgetState kind="unavailable" message="This transfer selection belongs to another recording or branch." /> : <>
      <div className="drill-header"><strong>Transfer breakdown</strong><span className="drill-day">{formatDateTime(drillAt)}</span>
        {drill.breakdown && drill.follow ? <span className="pill drill-following">Following playback</span>
          : <button type="button" className="button small" onClick={() => dispatchDrill({ type: 'open', breakdown: 'parts', hour: drillAt, playing: true, playhead })}>Follow playback</button>}
        <button type="button" className="button small" onClick={pin}>Pin as widget</button></div>{breakdown}</>
      : <DrillHost mode={options.drill === 'popout' ? 'popout' : 'inline'} open={drill.breakdown != null} anchor={drillAnchor} title="Transfer breakdown"
        day={formatDateTime(drillAt)} following={drill.follow} canFollow={playhead != null} onClose={closeDrill}
        onFollow={() => dispatchDrill({ type: 'follow' })} onPin={pin} main={main}>{breakdown}</DrillHost>}
    {menu.menu}
  </div>;
}
