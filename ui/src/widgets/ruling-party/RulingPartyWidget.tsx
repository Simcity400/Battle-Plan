import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { badgeRows } from '../../charts/badges';
import { dayLabel, drawStack } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import Legend from '../../charts/Legend';
import { categoricalPalette } from '../../charts/palette';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import type { TooltipRow } from '../../charts/tooltip';
import type { PlaybackMode } from '../../charts/view';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import type { PartyHistoryReply } from '../../generated/dashboard';
import { readTokens, subscribeTokens } from '../../charts/tokens';
import { OPTIONS } from './definition';
import { bands, buildStack, formatPopularity, partyBadges, rawAt, rulingAt, rulingFirst, stackedSegments, validatePartyHistoryReply } from './partyData';
import './rulingParty.css';

const NO_WARNINGS: string[] = [];

/** One country's politics over time: party popularity as stacked bands in each
 * ideology's own colour, with changes of ruling party and elections as badges. */
export default function RulingPartyWidget({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const request = track && tag ? { sourceId: track.source.id, revision: track.source.revision, product: 'party-history',
    params: { branchId: track.branch.id, tag } } : null;
  const { data, error, retry } = useProduct<PartyHistoryReply>(runtime.client, request, active, validatePartyHistoryReply);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const following = widget.time.mode === 'follow' || widget.time.mode === 'pin';
  const playhead = window && following ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  const head = playhead ?? extent[1];
  // Fallback colours are solved against the current theme, so they follow a theme change.
  const [surface, setSurface] = useState(() => readTokens().surface);
  useEffect(() => subscribeTokens(tokens => setSurface(tokens.surface)), []);
  // The party ruling at the moment shown goes first: the bottom band and the first legend entry.
  const rulingShown = data ? rulingAt(data, Math.floor(head)) : null;
  const list = useMemo(() => data ? rulingFirst(bands(data, categoricalPalette(surface)), rulingShown) : [], [data, surface, rulingShown]);
  // Built once per reply; each frame only slices and thins it.
  const stack = useMemo(() => data ? buildStack(list, data.decimals) : null, [data, list]);
  const names = useMemo(() => new Map(list.map(band => [band.token, band.name])), [list]);
  const badges = useMemo(() => data && options.changes ? partyBadges(data, token => names.get(token) ?? token) : [], [data, names, options.changes]);
  const menu = useContextMenu('Chart actions');
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, onDiagnostics]);

  const layers = useMemo<ChartLayer[]>(() => data ? [{
    draw: (ctx, frame, tokens, state) => {
      const to = Math.min(frame.x[1], state.playhead ?? frame.x[1]);
      const hoursPerPixel = (frame.x[1] - frame.x[0]) / Math.max(1, frame.plotWidth);
      for (const segment of stackedSegments(stack!, frame.x[0], to, hoursPerPixel / 2)) {
        drawStack(ctx, frame, tokens, segment.hours, segment.tops, list.map(band => band.color));
      }
    },
    extent: () => [0, 100],
  }] : [], [data, list, stack]);

  if (!track || !tag) return <WidgetState kind="empty" icon="palette" message="Load a campaign and choose a country to see its politics." />;
  if (playhead != null && (playhead < track.branch.firstHour || playhead > track.branch.lastHour)) return <WidgetState kind="unavailable" message="This time is outside the country’s recorded branch." />;
  if ((widget.time.mode === 'range' || widget.time.mode === 'pin') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading parties…" />;
  if (!list.length) return <WidgetState kind="unavailable" message="No parties were recorded for this country in this branch." />;

  const ruling = rulingAt(data, Math.floor(head));
  const rulingRaw = ruling ? rawAt(list.find(band => band.token === ruling)!.series, Math.floor(head)) : null;
  return <div className="ruling-party">
    {headerSlot && ruling && createPortal(<span className="pill">{names.get(ruling) ?? ruling}{rulingRaw ? ` · ${formatPopularity(rulingRaw, data.decimals)}` : ''}</span>, headerSlot)}
    {/* Names only, as signed off; each popularity is still read out to assistive technology. */}
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
          const rows: TooltipRow[] = list.map(band => ({ band, raw: rawAt(band.series, hour) }))
            .sort((a, b) => Number(b.raw ?? -1) - Number(a.raw ?? -1))
            .map(({ band, raw }) => ({ label: band.token === rulingAt(data, hour) ? `${band.name} (ruling)` : band.name, swatch: band.color,
              value: raw == null ? 'Not recorded' : formatPopularity(raw, data.decimals), tone: raw == null ? 'muted' as const : undefined }));
          rows.push(...badgeRows(hovered));
          return { title: dayLabel(hour), rows };
        }}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    </div>
    {menu.menu}
  </div>;
}
