import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles.ts';
import { useEffect, useMemo, useState, useRef } from 'react';
import { markIndex } from '../../charts/markSummary';
import type { MarkSummary } from '../../charts/markSummary';
import { usePreparedGovernments } from './preparedGovernments';
import WindowedRows from '../../charts/WindowedRows';
import { useParityRows } from '../../dashboard/parityRows';
import { useContextMenu } from '../../components/ContextMenu';
import ArtFallback from '../../components/ArtFallback';
import WidgetState from '../../components/WidgetState';
import { useHeldFailure } from '../../dashboard/heldFailures';
import { badgeRows } from '../../charts/badges';
import { dayLabel, drawBands } from '../../charts/draw';
import { jumpItem, rowJumpProps } from '../../charts/jump';
import Legend from '../../charts/Legend';
import { categoricalPalette } from '../../charts/palette';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import type { TooltipRow } from '../../charts/tooltip';
import { shownUntil } from '../../charts/view';
import type { PlaybackMode } from '../../charts/view';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import type { PartyHistoryReply, PoliticsLedgerReply } from '../../generated/dashboard';
import { readTokens, subscribeTokens } from '../../charts/tokens';
import { OPTIONS } from './definition';
import { bands, buildStack, formatPopularity, leaderAt, leaderShown, partyBadges, partyNameAt, rawAt, rulingAt, rulingFirst, stackedSegments, validatePartyHistoryReply } from './partyData';
import { governmentBadges, governmentEventRows, governmentRows, validateGovernmentLedger, visibleGovernmentChanges } from './governmentData';
import { drawApproximateSpans, summaryStack, summaryFits } from '../../charts/chartSummary';
import { forEachCompactStackSegment } from '../../charts/stack';
import PartyEventList from './PartyEventList';
import { partyMarkSummaries, usePreparedParty } from './preparedParty';
import { usePartyParity } from './partyParity';
import CountryPeople from '../country-people/CountryPeople';
import './rulingParty.css';
import { motionOf } from '../../charts/easing.ts';

const NO_WARNINGS: string[] = [];

/** One country's politics over time: the ruling party's leader, then party
 * popularity as stacked bands in each ideology's own colour, with changes of
 * ruling party and elections as badges. */
export default function RulingPartyWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const parityRoot = useRef<HTMLDivElement>(null);
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'party-history',
    params: { branchId: track.branch.id, tag } } : null;

  const governmentRequest = !runtime.client.preparedQuery && track && tag && options.changes ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'politics-ledger', params: { branchId: track.branch.id, tag, view: 'governments' } } : null;
  const referenceGovernments = useProduct<PoliticsLedgerReply>(runtime.client, governmentRequest, active,
    reply => validateGovernmentLedger(reply, governmentRequest?.params));
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const head = playhead ?? extent[1];
  const [hover, setHover] = useState<number | null>(null);
  const party = usePreparedParty(runtime.client, track?.source, track?.branch.id, tag, head, hover, active,
    [...(options.changes ? ['ruling', 'election'] : []), ...(options.leaders ? ['leader'] : [])]);
  const legacy = useProduct<PartyHistoryReply>(runtime.client, request, active && !party.enabled, validatePartyHistoryReply);
  const reader = party.enabled ? party : legacy;
  const { data, retry } = reader;
  const parity = import.meta.env.MODE === 'parity' ? usePartyParity(runtime.client, track?.source, track?.branch.id, tag, extent, 'Party history values', legacy.data) : undefined;
  const detailsThrough = shownUntil(playhead, (options.playback as PlaybackMode) ?? 'draw-in', options.afterPlayhead === true) ?? extent[1];
  const governments = usePreparedGovernments(runtime.client, track?.source, track?.branch.id, options.changes ? tag : undefined, extent, detailsThrough, active);
  const governmentReply = governments.enabled ? governments : referenceGovernments;
  const keyboardOwner = useMemo(() => ({ party: party.owner, governments: governments.owner }),
    [party.owner, governments.owner, governments.view.from, governments.view.to, governments.view.width, detailsThrough]);
  const navigation = useRef<AbortController | null>(null);
  const navigationOwner = useRef(keyboardOwner); navigationOwner.current = keyboardOwner;
  const [keyboard, setKeyboard] = useState<{ owner: typeof keyboardOwner; hour: number | null; revision: number }>();
  useEffect(() => () => navigation.current?.abort(), [keyboardOwner, active]);
  // Fallback colours are solved against the current theme, so they follow a theme change.
  const [surface, setSurface] = useState(() => readTokens().surface);
  useEffect(() => subscribeTokens(tokens => setSurface(tokens.surface)), []);
  // The party ruling at the moment shown goes first: the bottom band and the first legend entry.
  const presentationHour = party.enabled ? party.presentationHour : Math.floor(head);
  // A failed seek must never display another hour's held reply.
  const error = useHeldFailure(reader.error, data !== undefined && presentationHour === Math.floor(head), retry);
  const rulingShown = data ? rulingAt(data, presentationHour) : null;
  const list = useMemo(() => data ? rulingFirst(bands(data, categoricalPalette(surface)), rulingShown) : [], [data, surface, rulingShown]);
  // Built once per reply; each frame only slices and thins it.
  const plotted = useMemo(() => party.enabled && party.summary ? rulingFirst(bands(party.summary.reply, categoricalPalette(surface)), rulingShown) : list, [party.enabled, party.summary, surface, rulingShown, list]);
  const stack = useMemo(() => data ? buildStack(plotted, data.decimals) : null, [data, plotted]);
  const dense = useMemo(() => party.summary && party.summary.summaries.some(line => !line.summary.runs)
    ? summaryStack(plotted.map(band => party.summary!.summaries.find(line => line.token === band.token)!.summary)) : null, [party.summary, plotted]);
  const approximate = useMemo(() => party.summary?.summaries.flatMap(line => line.summary.approximate ?? []), [party.summary]);
  const names = useMemo(() => {
    const names = new Map((data?.ideologies ?? []).map(ideology => [ideology.token, contentTitle(ideology.name, ideology.token, 'Ideology name unavailable')]));
    for (const band of list) names.set(band.token, band.name);
    return names;
  }, [data, list]);
  const badges = useMemo(() => {
    if (!data) return [];
    const native = visibleGovernmentChanges(governments.enabled && !governments.exact ? [] : governmentReply.data?.governments ?? [], detailsThrough);
    const source = party.enabled ? party.summary?.reply : data;
    // Fields the chart's mark summaries already show (`markSummaries`) are not badges too.
    const summarized = new Set(party.summary?.marks.filter(mark=>!mark.exact || governments.enabled && !governments.exact).map(mark=>mark.field));
    const all = source ? partyBadges(source, token => names.get(token) ?? 'Ideology name unavailable', options.leaders === true, native).filter(badge=>!party.enabled || !summarized.has(badge.glyph==='\u2713' ? 'election' : badge.glyph==='\u2654' ? 'leader' : 'ruling')) : [];
    return options.changes ? [...all, ...governmentBadges(native, token => names.get(token) ?? 'Ideology name unavailable', detailsThrough)].sort((a, b) => a.hour - b.hour)
      : all.filter(badge => badge.glyph === '♔');
  }, [data, party.enabled, party.summary, names, options.changes, options.leaders, governmentReply.data, detailsThrough, governments.enabled, governments.exact]);
  const native = visibleGovernmentChanges(governments.enabled && !governments.exact ? [] : governmentReply.data?.governments ?? [], detailsThrough);
  const censusBadges = party.summary ? partyBadges(party.summary.reply,token=>names.get(token) ?? 'Ideology name unavailable',options.leaders===true,native) : [];
  const pageBadges = party.events ? partyBadges(party.events.reply,token=>names.get(token) ?? 'Ideology name unavailable',options.leaders===true,native) : [];
  const reachedBadges = party.reached ? partyBadges(party.reached.reply,token=>names.get(token) ?? 'Ideology name unavailable',options.leaders===true,native) : [];
  const supplementalIndex = useMemo(() => markIndex(governmentBadges(governments.data?.governments ?? [], token => names.get(token) ?? 'Ideology name unavailable', detailsThrough)), [governments.data, names, detailsThrough]);
  const markSummaries = useMemo<MarkSummary[] | undefined>(() => {
    if (!party.enabled) return undefined;
    const summaries = partyMarkSummaries(party.summary, censusBadges, governments.enabled && !governments.exact) ?? [];
    if (!governments.enabled || governments.exact) return summaries.length ? summaries : undefined;
    const native = governments.data?.governmentMarks;
    if (native) summaries.push({ ...native, clusters: native.clusters.map(cluster => ({ ...cluster, tag: 'native',
      ...(cluster.count === '1' ? { badge: { hour: cluster.firstHour, glyph: '\u21c6', label: 'Government change' } } : {}) })) });
    const tail = supplementalIndex.summary(governments.view.from, governments.view.to, governments.view.width);
    summaries.push({ ...tail, clusters: tail.clusters.map(cluster => ({ ...cluster, tag: 'tail' })) });
    return summaries;
  }, [party.enabled, party.summary, censusBadges, governments.enabled, governments.exact, governments.data, governments.view.from, governments.view.to, governments.view.width, supplementalIndex]);
  const exactCauses = governments.enabled ? governments.details?.governments ?? governments.data?.governments ?? [] : referenceGovernments.data?.governments ?? [];
  const parityCauses = governments.enabled ? governments.parityRows?.governments ?? [] : referenceGovernments.data?.governments ?? [];
  useParityRows(parityRoot, () => ({ kind: 'chart', label: 'Government execution causes', scope: 'displayed-hour', hour: head,
    complete: !!data && (!options.changes || (governments.enabled ? !!governments.parityRows : !!referenceGovernments.data)), total: visibleGovernmentChanges(parityCauses, detailsThrough).length,
    rows: visibleGovernmentChanges(parityCauses, detailsThrough).map(row => ({ hour: row.hour, readHour: row.readHour,
      country: row.country, instance: row.instance, epoch: row.epoch, sequence: row.sequence, cause: row.cause,
      before: row.beforeToken ?? null, after: row.afterToken ?? null, covered: row.covered })) }), {
    prepare: governments.prepareParity, ready: () => !governments.enabled || !!governments.parityRows, error: () => governments.parityError ?? null,
  });
  const [peopleWarnings, setPeopleWarnings] = useState<string[]>([]);
  const menu = useContextMenu('Chart actions');
  useEffect(() => {
    const warnings = [...peopleWarnings, ...(data?.warnings ?? []), ...(governmentReply.data?.warnings ?? []),
      ...(governmentReply.error ? [`Government causes could not be loaded: ${governmentReply.error}`] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, governmentReply.data, governmentReply.error, peopleWarnings, onDiagnostics]);

  const layers = useMemo<ChartLayer[]>(() => data ? [{
    draw: (ctx, frame, tokens, state) => {
      const to = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      const hoursPerPixel = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth);
      if (party.enabled && (!party.summary || party.summary.summaries.some(line => !summaryFits(frame, line.summary, to)))) return;
      if (dense) {
        if (party.summary?.summaries.some(line => !summaryFits(frame, line.summary, to))) return;
        drawApproximateSpans(ctx, frame, dense.approximate, to, () => {
          forEachCompactStackSegment(dense.model,frame.x[0],to,hoursPerPixel/2,segment => drawBands(ctx,frame,tokens,segment.hours,segment.bands,plotted.map(band=>band.color)),dense.sample);
        });
        return;
      }
      // Shared clipping unions the per-party spans without duplicating their style.
      drawApproximateSpans(ctx, frame, approximate, to, () => {
        for (const segment of stackedSegments(stack!, frame.x[0], to, hoursPerPixel / 2)) {
          drawBands(ctx, frame, tokens, segment.hours, segment.bands, plotted.map(band => band.color));
        }
      });
    },
    extent: () => [0, 100],
  }] : [], [data, plotted, stack, dense, approximate, party.enabled, party.summary]);

  if (!track || !tag) return <WidgetState kind="empty" icon="palette" message="Load a campaign and choose a country to see its politics." />;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading parties…" />;
  if (!list.length && !data.leaders.length) return <WidgetState kind="unavailable" message="Party data is unavailable." />;

  const ruling = rulingAt(data, presentationHour);
  const rulingBand = list.find(band => band.token === ruling);
  const rulingRaw = rulingBand ? rawAt(rulingBand.series, Math.floor(presentationHour)) : null;
  const leaderRun = leaderAt(data, presentationHour);
  const leader = leaderShown(data, leaderRun);
  // Portrait ringed in the ruling ideology's colour (grey when not
  // known), the leader's name, then ideology · party · share.
  // The party as the game names it ("NSDAP"); left out when that is not known.
  const detail = [leader.ideology, partyNameAt(data, presentationHour), rulingRaw ? formatPopularity(rulingRaw, data.decimals) : null].filter(Boolean).join(' · ');
  const header = <div className="ruling-party-leader" ref={parity}>
    <span className="ruling-party-portrait" style={rulingBand ? { borderColor: rulingBand.color } : undefined}>
      {leader.portrait ? <img src={leader.portrait} alt="" /> : <ArtFallback icon="helmet" />}
    </span>
    <div className="ruling-party-leader-text">
      <strong className={leader.known ? undefined : 'is-notice'}>{leader.name}</strong>
      {detail && <span data-tooltip={contentTitleHint(leaderRun?.ideology)}>{detail}</span>}
    </div>
  </div>;
  // A recorded leader still shows when no party popularity was recorded.
  if (!list.length) return <div className="ruling-party" ref={parityRoot}>{header}
    <WidgetState kind="unavailable" message="Party popularity is unavailable." />
  </div>;
  return <div className="ruling-party" ref={parityRoot}>
    {header}
    <CountryPeople runtime={runtime} track={track} tag={tag} hour={presentationHour} active={active} view="politics" onWarnings={setPeopleWarnings} />
    {/* Names only; each popularity is still read out to assistive technology. */}
    <Legend label="Parties" items={list.map(band => {
      const raw = rawAt(band.series, Math.floor(presentationHour));
      return { id: band.token, label: band.name, color: band.color, spokenValue: raw == null ? 'not recorded' : formatPopularity(raw, data.decimals) };
    })} />
    <div className="ruling-party-chart">
      <TimeChart label={`Party popularity in ${tag}`} extent={extent} playhead={playhead}
        playback={(options.playback as PlaybackMode) ?? 'draw-in'} afterPlayhead={options.afterPlayhead === true} windowHours={365 * 24}
        hoursPerSecond={motionOf(runtime.clock.getSnapshot())} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: 0, yMax: 100, log: false }} layers={layers} active={active} revision={`${head}:${data.branchId}`}
        onSummaryView={party.enabled ? view => { party.onSummaryView(view); governments.onSummaryView(view); } : undefined}
        onHover={party.enabled ? hour => { setHover(hour); if (governments.enabled) governments.onHover(hour); } : undefined}
        onNavigateHour={party.enabled ? (key, from, bounds, until) => {
          navigation.current?.abort(); const controller = new AbortController(); navigation.current = controller; const owner = keyboardOwner;
          const visibleBounds: [number, number] = [bounds[0], Math.min(bounds[1], until ?? bounds[1])];
          void Promise.all([party.navigate(key, from, visibleBounds, until, controller.signal),
            governments.enabled ? governments.navigate(key, from, visibleBounds, controller.signal) : Promise.resolve(null)]).then(([census, native]) => {
            const forward = key === 'Home' || key === 'ArrowRight';
            const candidates = [census, native].filter((hour): hour is number => hour !== null && (from === null || hour !== from));
            const hour = candidates.length ? (forward ? Math.min(...candidates) : Math.max(...candidates)) : census ?? native;
            if (!controller.signal.aborted && navigationOwner.current === owner) {
              setHover(hour); if (governments.enabled) governments.onHover(hour);
              setKeyboard(previous => ({ owner, hour, revision: (previous?.revision ?? 0) + 1 }));
            }
          }).catch(() => { if (!controller.signal.aborted) { party.retry(); if (governments.enabled) governments.retry(); } });
        } : undefined}
        keyboardHour={keyboard?.owner === keyboardOwner ? keyboard.hour : undefined} keyboardRevision={keyboard?.owner === keyboardOwner ? keyboard.revision : undefined}
        pickOwner={keyboardOwner}
        onMarkHover={party.enabled ? cluster => {
          if (cluster?.tag === 'native' || cluster?.tag === 'tail') governments.onMarkHover(cluster);
          else party.onMarkHover(cluster);
        } : undefined}
        markSummaries={markSummaries}
        badgesAtHour={party.enabled ? hour => [...reachedBadges, ...governmentBadges(native, token => names.get(token) ?? 'Ideology name unavailable', detailsThrough)].filter(badge => badge.hour === hour) : undefined}
        markTooltip={cluster => cluster.tag === 'native' || cluster.tag === 'tail'
          ? { title: `${cluster.count} government changes`, rows: [
            { label: `${dayLabel(cluster.firstHour)} - ${dayLabel(cluster.lastHour)}` },
            ...(cluster.badge ? badgeRows([cluster.badge]) : []),
            ...(governments.cluster?.tag === cluster.tag && governments.cluster?.firstOrdinal === cluster.firstOrdinal
              ? (governments.rows ?? []).flatMap(row => governmentEventRows(row, token => names.get(token) ?? 'Ideology name unavailable')) : []),
          ] }
          : { title: dayLabel(cluster.firstHour), rows: party.events ? badgeRows(pageBadges) : [{ label: 'Events', value: 'Loading...' }] }}
        formatValue={value => `${Math.round(value)}%`} badges={badges.length ? badges : undefined}
        tooltip={(hour, _frame, hovered) => {
          if (party.enabled && !party.loaded(hour)) return {title:dayLabel(hour),rows:[{label:'Parties',value:'Loading...',tone:'muted'}]};
          const shown = leaderShown(data, leaderAt(data, hour));
          const name = partyNameAt(data, hour);
          const partyLabel = name === undefined ? null : name ?? 'Not known';
          const rows: TooltipRow[] = [{ label: 'Leader', value: shown.name, tone: shown.known ? undefined : 'muted' as const }];
          if (shown.ideology || partyLabel) rows.push({ label: 'Ideology · party', value: [shown.ideology ?? 'Not recorded', partyLabel ?? 'Not recorded'].join(' · ') });
          rows.push(...list.map(band => ({ band, raw: rawAt(band.series, hour) }))
            .sort((a, b) => Number(b.raw ?? -1) - Number(a.raw ?? -1))
            .map(({ band, raw }) => ({ label: band.token === rulingAt(data, hour) ? `${band.name} (ruling)` : band.name, swatch: band.color,
              value: raw == null ? 'Not recorded' : formatPopularity(raw, data.decimals), tone: raw == null ? 'muted' as const : undefined })));
          if (options.changes) rows.push(...governmentRows(exactCauses, hour, detailsThrough, token => names.get(token) ?? 'Ideology name unavailable'));
          rows.push(...badgeRows(hovered));
          return { title: dayLabel(hour), rows };
        }}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    </div>
    <PartyEventList prepared={party} badges={pageBadges} />
    {governments.cluster && <section className="ruling-party-changes" aria-label="Government changes">
      <div className="chart-line-key"><strong>Government changes</strong><button type="button" className="button small" onClick={governments.close}>Close</button></div>
      {!governments.rows ? <WidgetState kind="loading" label="Loading government changes..." />
        : <WindowedRows label="Recorded government changes" items={governments.rows.map(row => ({ key: JSON.stringify([row.country, row.instance, row.epoch, row.sequence]), item: row }))} rowHeight={64}>
          {row => <div tabIndex={0} {...rowJumpProps(runtime, track.id, row.item.hour, menu.open)}>{governmentEventRows(row.item, token => names.get(token) ?? 'Ideology name unavailable').map((detail, index) => <div key={index}>{detail.label}{detail.value ? `: ${detail.value}` : ''}</div>)}</div>}
        </WindowedRows>}
      <div className="chart-line-key"><button type="button" className="button small" disabled={!governments.previous} onClick={governments.previous}>Previous</button>
        <span>{governments.cluster.count} recorded changes</span><button type="button" className="button small" disabled={!governments.next} onClick={governments.next}>Next</button></div>
    </section>}
    {governmentReply.error && <WidgetState kind="error" what="government changes" failure={governmentReply.error} onRetry={governmentReply.retry} />}
    {menu.menu}
  </div>;
}
