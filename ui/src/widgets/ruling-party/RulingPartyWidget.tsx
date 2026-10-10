import { useEffect, useMemo, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { badgeRows } from '../../charts/badges';
import { dayLabel, drawBands } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
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
import { governmentBadges, governmentRows, validateGovernmentLedger, visibleGovernmentChanges } from './governmentData';
import './rulingParty.css';

const NO_WARNINGS: string[] = [];

/** One country's politics over time: the ruling party's leader, then party
 * popularity as stacked bands in each ideology's own colour, with changes of
 * ruling party and elections as badges. */
export default function RulingPartyWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'party-history',
    params: { branchId: track.branch.id, tag } } : null;
  const { data, error, retry } = useProduct<PartyHistoryReply>(runtime.client, request, active, validatePartyHistoryReply);
  const governmentRequest = track && tag && options.changes ? { sourceId: track.source.id, revision: track.source.revision,
    product: 'politics-ledger', params: { branchId: track.branch.id, tag, view: 'governments' } } : null;
  const governments = useProduct<PoliticsLedgerReply>(runtime.client, governmentRequest, active,
    reply => validateGovernmentLedger(reply, governmentRequest?.params));
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const head = playhead ?? extent[1];
  const detailsThrough = shownUntil(playhead, (options.playback as PlaybackMode) ?? 'draw-in', options.afterPlayhead === true) ?? extent[1];
  // Fallback colours are solved against the current theme, so they follow a theme change.
  const [surface, setSurface] = useState(() => readTokens().surface);
  useEffect(() => subscribeTokens(tokens => setSurface(tokens.surface)), []);
  // The party ruling at the moment shown goes first: the bottom band and the first legend entry.
  const rulingShown = data ? rulingAt(data, Math.floor(head)) : null;
  const list = useMemo(() => data ? rulingFirst(bands(data, categoricalPalette(surface)), rulingShown) : [], [data, surface, rulingShown]);
  // Built once per reply; each frame only slices and thins it.
  const stack = useMemo(() => data ? buildStack(list, data.decimals) : null, [data, list]);
  const names = useMemo(() => {
    const names = new Map((data?.ideologies ?? []).map(ideology => [ideology.token, ideology.name ?? ideology.token]));
    for (const band of list) names.set(band.token, band.name);
    return names;
  }, [data, list]);
  const badges = useMemo(() => {
    if (!data) return [];
    const native = visibleGovernmentChanges(governments.data?.governments ?? [], detailsThrough);
    const all = partyBadges(data, token => names.get(token) ?? token, options.leaders === true, native);
    return options.changes ? [...all, ...governmentBadges(native, token => names.get(token) ?? token, detailsThrough)].sort((a, b) => a.hour - b.hour)
      : all.filter(badge => badge.glyph === '♔');
  }, [data, names, options.changes, options.leaders, governments.data, detailsThrough]);
  const menu = useContextMenu('Chart actions');
  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(governments.data?.warnings ?? []),
      ...(governments.error ? [`Government causes could not be loaded: ${governments.error}`] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, governments.data, governments.error, onDiagnostics]);

  const layers = useMemo<ChartLayer[]>(() => data ? [{
    draw: (ctx, frame, tokens, state) => {
      const to = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      const hoursPerPixel = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth);
      for (const segment of stackedSegments(stack!, frame.x[0], to, hoursPerPixel / 2)) {
        drawBands(ctx, frame, tokens, segment.hours, segment.bands, list.map(band => band.color));
      }
    },
    extent: () => [0, 100],
  }] : [], [data, list, stack]);

  if (!track || !tag) return <WidgetState kind="empty" icon="palette" message="Load a campaign and choose a country to see its politics." />;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="unavailable" message="This time is outside the country’s recorded branch." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading parties…" />;
  if (!list.length && !data.leaders.length) return <WidgetState kind="unavailable" message="No parties were recorded for this country in this branch." />;

  const ruling = rulingAt(data, Math.floor(head));
  const rulingBand = list.find(band => band.token === ruling);
  const rulingRaw = rulingBand ? rawAt(rulingBand.series, Math.floor(head)) : null;
  const leader = leaderShown(data, leaderAt(data, Math.floor(head)));
  // Portrait ringed in the ruling ideology's colour (grey when not
  // known), the leader's name, then ideology · party · share.
  // The party as the game names it ("NSDAP"); left out when that is not known.
  const detail = [leader.ideology, partyNameAt(data, Math.floor(head)), rulingRaw ? formatPopularity(rulingRaw, data.decimals) : null].filter(Boolean).join(' · ');
  const header = <div className="ruling-party-leader">
    {/* Why a portrait is missing is on the shared tooltip, reachable by keyboard too. */}
    <span className="ruling-party-portrait" style={rulingBand ? { borderColor: rulingBand.color } : undefined}
      {...(leader.portraitMissing ? { tabIndex: 0, role: 'img', 'aria-label': `Portrait not shown: ${leader.portraitMissing}`, 'data-tooltip': leader.portraitMissing } : {})}>
      {leader.portrait ? <img src={leader.portrait} alt="" /> : <span className="ruling-party-silhouette" aria-hidden="true" />}
    </span>
    <div className="ruling-party-leader-text">
      <strong className={leader.known ? undefined : 'is-notice'}>{leader.name}</strong>
      {detail && <span>{detail}</span>}
    </div>
  </div>;
  // A recorded leader still shows when no party popularity was recorded.
  if (!list.length) return <div className="ruling-party">{header}
    <WidgetState kind="unavailable" message="No party popularity was recorded for this country in this branch." />
  </div>;
  return <div className="ruling-party">
    {header}
    {/* Names only; each popularity is still read out to assistive technology. */}
    <Legend label="Parties" items={list.map(band => {
      const raw = rawAt(band.series, Math.floor(head));
      return { id: band.token, label: band.name, color: band.color, spokenValue: raw == null ? 'not recorded' : formatPopularity(raw, data.decimals) };
    })} />
    <div className="ruling-party-chart">
      <TimeChart label={`Party popularity in ${tag}`} extent={extent} playhead={playhead}
        playback={(options.playback as PlaybackMode) ?? 'draw-in'} afterPlayhead={options.afterPlayhead === true} windowHours={365 * 24}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: 0, yMax: 100, log: false }} layers={layers} active={active} revision={`${head}:${data.branchId}`}
        formatValue={value => `${Math.round(value)}%`} badges={badges.length ? badges : undefined}
        tooltip={(hour, _frame, hovered) => {
          const shown = leaderShown(data, leaderAt(data, hour));
          const name = partyNameAt(data, hour);
          const party = name === undefined ? null : name ?? 'Not known';
          const rows: TooltipRow[] = [{ label: 'Leader', value: shown.name, tone: shown.known ? undefined : 'muted' as const }];
          if (shown.ideology || party) rows.push({ label: 'Ideology · party', value: [shown.ideology ?? 'Not recorded', party ?? 'Not recorded'].join(' · ') });
          if (shown.portraitMissing) rows.push({ label: 'Portrait', value: `Not shown: ${shown.portraitMissing}`, tone: 'muted' });
          rows.push(...list.map(band => ({ band, raw: rawAt(band.series, hour) }))
            .sort((a, b) => Number(b.raw ?? -1) - Number(a.raw ?? -1))
            .map(({ band, raw }) => ({ label: band.token === rulingAt(data, hour) ? `${band.name} (ruling)` : band.name, swatch: band.color,
              value: raw == null ? 'Not recorded' : formatPopularity(raw, data.decimals), tone: raw == null ? 'muted' as const : undefined })));
          if (options.changes) rows.push(...governmentRows(governments.data?.governments ?? [], hour, detailsThrough, token => names.get(token) ?? token));
          rows.push(...badgeRows(hovered));
          return { title: dayLabel(hour), rows };
        }}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    </div>
    {governments.error && <button type="button" onClick={governments.retry}>Retry government causes</button>}
    {menu.menu}
  </div>;
}
