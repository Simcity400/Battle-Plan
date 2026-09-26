import { useEffect, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { SourceFlag } from '../../components/CountryFlag';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel, drawEndLabels, drawEventTicks, drawLine } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { layoutEndLabels } from '../../charts/labels';
import Legend from '../../charts/Legend';
import { categoricalPalette, nudgeApart } from '../../charts/palette';
import { extentOf } from '../../charts/series';
import type { Run } from '../../charts/series';
import TimeChart from '../../charts/TimeChart';
import type { ChartLayer } from '../../charts/TimeChart';
import { readTokens, subscribeTokens } from '../../charts/tokens';
import type { ChartTokens } from '../../charts/tokens';
import type { TooltipRow } from '../../charts/tooltip';
import type { PlaybackMode } from '../../charts/view';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { membershipChanges } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import type { CountryStyle, CountryStyleReply, PoliticalPowerReply } from '../../generated/dashboard';
import { parseColor, toHex } from '../../shell/appearance/color';
import { OPTIONS } from './definition';
import { clipRuns, formatBalance, isMember, nearestDrop, netSpend, rawAt, resolveCountries, toRuns, validatePoliticalPowerReply } from './politicalPowerData';
import './politicalPower.css';

const NO_WARNINGS: string[] = [];
/** Country names and colours are read at most once per this many hours of playback. */
const STYLE_STEP = 30 * 24;
/** Hovering within this many pixels of a spend tick snaps to it. */
const SNAP_PX = 6;

function useTokens(): ChartTokens {
  const [tokens, setTokens] = useState(readTokens);
  useEffect(() => subscribeTokens(setTokens), []);
  return tokens;
}

/** Political power balance over time, one line per country, with net-spend ticks.
 * Daily income is not recorded, so spends are the falls in the balance. */
export default function PoliticalPowerWidget({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const tokens = useTokens();
  const options = resolveOptions(OPTIONS, widget.options);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const lastHour = track?.branch.lastHour ?? 0;
  const groups = useCountryGroups(runtime, track, active && widget.countries.kind === 'group');
  const fallback = defaultCountryTag(track);
  const resolved = useMemo(() => resolveCountries(widget.countries, fallback ? [fallback] : [], groups.data, lastHour), [widget.countries, fallback, groups.data, lastHour]);
  const request = track && resolved.tags.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'political-power',
    params: { branchId: track.branch.id, tags: resolved.tags } } : null;
  const { data, error, retry } = useProduct<PoliticalPowerReply>(runtime.client, request, active, validatePoliticalPowerReply);

  const following = widget.time.mode === 'follow';
  const pinned = widget.time.mode === 'pin';
  const playhead = window && (following || pinned) ? window.lastHour : null;
  const extent: [number, number] = track ? (widget.time.mode === 'range' && window ? [window.firstHour, window.lastHour] : [track.branch.firstHour, track.branch.lastHour]) : [0, 1];
  // Names and colours as they were at the moment shown: the playhead, the end of a
  // date range, or the branch end.
  const styleAt = playhead ?? (widget.time.mode === 'range' && window ? window.lastHour : lastHour);
  const styleHour = track ? Math.min(lastHour, track.branch.firstHour + Math.floor((styleAt - track.branch.firstHour) / STYLE_STEP) * STYLE_STEP) : 0;
  const styleRequest = track && resolved.tags.length ? { sourceId: track.source.id, revision: track.source.revision, product: 'country-styles',
    params: { branchId: track.branch.id, hour: styleHour, tags: resolved.tags } } : null;
  const styles = useProduct<CountryStyleReply>(runtime.client, styleRequest, active);
  // Keep each country's last known name and colour while the next hour's reply loads,
  // so lines never flash to fallback colours during playback.
  const [known, setKnown] = useState<ReadonlyMap<string, CountryStyle>>(new Map());
  useEffect(() => setKnown(new Map()), [track?.source.id, track?.source.revision, track?.branch.id]);
  useEffect(() => {
    if (styles.data) setKnown(previous => new Map([...previous, ...styles.data!.styles.map(style => [style.tag, style] as const)]));
  }, [styles.data]);

  useEffect(() => {
    const warnings = [...(data?.warnings ?? []), ...(styles.data?.warnings ?? []), ...(groups.data?.warnings ?? []),
      ...(resolved.dropped ? [`${resolved.dropped} more countries are left out; this widget draws at most ${resolved.tags.length}.`] : [])];
    onDiagnostics(warnings.length ? warnings : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [data, styles.data, groups.data, resolved.dropped, resolved.tags.length, onDiagnostics]);

  const series = useMemo(() => {
    if (!data) return [];
    const palette = categoricalPalette(tokens.surface);
    const raw = data.series.map((item, index) => {
      const style = known.get(item.tag);
      const recorded = style?.color ? { r: style.color[0], g: style.color[1], b: style.color[2] } : parseColor(palette[index % palette.length])!;
      const membership = resolved.membership?.get(item.tag);
      const runs = toRuns(item, data.decimals);
      // A country counted at each hour shows nothing (line, value, spend) outside its membership.
      const drops = item.drops.filter(drop => isMember(membership, drop.hour));
      const valueAt = (hour: number) => isMember(membership, hour) ? rawAt(item, hour) : null;
      return { tag: item.tag, name: style?.name ?? track?.source.metadata.countries.find(country => country.tag === item.tag)?.name ?? item.tag,
        color: recorded, drops, valueAt, runs: membership ? clipRuns(runs, membership) : runs, membership };
    });
    const adjusted = tokens.countryColours === 'fixed' ? nudgeApart(raw.map(item => item.color), parseColor(tokens.surface) ?? { r: 0, g: 0, b: 0 }) : raw.map(item => item.color);
    return raw.map((item, index) => ({ ...item, hex: toHex(adjusted[index]) }));
  }, [data, known, resolved.membership, tokens, track]);

  const changes = useMemo(() => series.flatMap(item => item.membership ? membershipChanges(item.membership, extent[0], extent[1], resolved.unobserved).map(change => ({ ...change, name: item.name, color: item.hex })) : []),
    [series, extent[0], extent[1], resolved.unobserved]);
  const menu = useContextMenu('Chart actions');
  const head = playhead ?? extent[1];

  const layers = useMemo<ChartLayer[]>(() => {
    const single = series.length === 1;
    const list: ChartLayer[] = series.map(item => ({
      draw: (ctx, frame, chartTokens, state) => { drawLine(ctx, frame, chartTokens, item.runs, item.hex, { area: single && options.areaFill === true, until: state.playhead ?? undefined }); },
      extent: (x: readonly [number, number]) => extentOf(item.runs as Run[], x[0], Math.min(x[1], playhead ?? x[1])),
    }));
    if (options.spendTicks) list.push({ draw: (ctx, frame, chartTokens) => {
      for (const item of series) drawEventTicks(ctx, frame, chartTokens, item.drops.map(drop => drop.hour).filter(hour => playhead == null || hour <= playhead), single ? chartTokens.muted : item.hex);
    } });
    return list;
  }, [series, options.areaFill, options.spendTicks, playhead]);

  if (!track) return <WidgetState kind="empty" icon="sparkle" message="Load a campaign to see political power." />;
  if ((pinned || widget.time.mode === 'range') && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (widget.countries.kind === 'group' && groups.error) return <WidgetState kind="error" message={groups.error} onRetry={groups.retry} />;
  if (resolved.reason) return <WidgetState kind="empty" message={resolved.reason} />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data || (widget.countries.kind === 'group' && !groups.data)) return <WidgetState kind="loading" label="Loading political power…" />;
  if (series.every(item => !item.runs.length)) return <WidgetState kind="unavailable" message="No political power was recorded for these countries in this branch." />;

  const decimals = data.decimals;
  const currentRaw = series.length === 1 ? series[0].valueAt(Math.floor(head)) : null;
  const hoursPerPixel = (frameWidth: number, span: number) => span / Math.max(1, frameWidth);
  return <div className="political-power">
    {headerSlot && currentRaw != null && createPortal(<span className="pill">{formatBalance(currentRaw, decimals)}</span>, headerSlot)}
    <Legend label="Countries" items={series.map(item => {
      const raw = item.valueAt(Math.floor(head));
      return { id: item.tag, label: item.name, color: item.hex, value: raw == null ? 'Not recorded' : formatBalance(raw, decimals),
        flag: <SourceFlag client={runtime.client} source={track.source} tag={item.tag} size="shield" /> };
    })} />
    <div className="political-power-chart">
      <TimeChart label={`Political power of ${series.map(item => item.name).join(', ')}`} extent={extent} playhead={playhead}
        playback={(options.playback as PlaybackMode) ?? 'draw-in'} windowHours={Number(options.windowDays ?? 365) * 24}
        hoursPerSecond={runtime.clock.getSnapshot().hoursPerSecond} branchStart={track.branch.firstHour}
        axes={{ xFrom: null, xTo: null, yMin: options.yMin as number | null, yMax: options.yMax as number | null, log: options.log === true }}
        padding={{ right: options.endLabels ? 128 : 14 }} layers={layers} active={active} revision={`${head}:${data.branchId}:${series.length}`}
        formatValue={value => Math.abs(value) >= 1000 ? `${Math.round(value / 100) / 10}k` : String(Math.round(value))}
        snap={(hour, frame) => {
          if (!options.spendTicks) return hour;
          const tolerance = SNAP_PX * hoursPerPixel(frame.plotWidth, frame.x[1] - frame.x[0]);
          // Only ticks that are drawn: inside the view and not after the playhead.
          const last = Math.min(frame.x[1], playhead ?? frame.x[1]);
          const visible = (drop: { hour: number }) => drop.hour >= frame.x[0] && drop.hour <= last;
          const nearest = series.map(item => nearestDrop(item.drops.filter(visible), hour, tolerance)).filter(drop => drop != null)
            .sort((a, b) => Math.abs(a!.hour - hour) - Math.abs(b!.hour - hour))[0];
          return nearest ? nearest.hour : hour;
        }}
        tooltip={(hour, frame) => {
          const tolerance = SNAP_PX * hoursPerPixel(frame.plotWidth, frame.x[1] - frame.x[0]);
          const rows: TooltipRow[] = series.map(item => {
            const raw = item.valueAt(hour);
            return { label: item.name, swatch: item.hex, value: raw == null ? 'Not recorded' : formatBalance(raw, decimals), tone: raw == null ? 'muted' as const : undefined };
          });
          for (const item of series) {
            const drop = item.drops.find(entry => entry.hour === hour);
            if (drop) rows.push({ label: `${series.length > 1 ? `${item.name}: ` : ''}Spent (net)`, value: netSpend(drop, decimals), tone: 'negative', divider: true });
          }
          for (const change of changes.filter(entry => Math.abs(entry.hour - hour) <= tolerance)) {
            rows.push({ label: `${change.name} ${change.change}`, swatch: change.color, divider: true });
          }
          return { title: dayLabel(hour), rows, note: rows.some(row => row.tone === 'negative') ? 'Daily income is not recorded, so a spend is the fall in the balance.' : undefined };
        }}
        overlay={(ctx, frame, chartTokens) => {
          for (const change of changes) {
            if (change.hour < frame.x[0] || change.hour > Math.min(frame.x[1], head)) continue;
            const x = frame.toX(change.hour);
            ctx.fillStyle = change.color;
            ctx.beginPath(); ctx.moveTo(x, frame.plotTop + 1); ctx.lineTo(x - 4, frame.plotTop - 6); ctx.lineTo(x + 4, frame.plotTop - 6); ctx.fill();
          }
          if (!options.endLabels) return;
          const at = Math.min(head, frame.x[1]);
          const ends = series.flatMap(item => {
            const raw = item.valueAt(Math.floor(at));
            return raw == null ? [] : [{ id: item.tag, y: frame.toY(Number(raw) / 10 ** decimals), height: 14, text: `${item.name} ${formatBalance(raw, decimals)}`, color: item.hex }];
          });
          const placed = layoutEndLabels(ends, frame.plotTop, frame.plotTop + frame.plotHeight);
          drawEndLabels(ctx, frame, chartTokens, ends.map(end => ({ text: end.text, y: placed.get(end.id)!, from: end.y, color: end.color })));
        }}
        onContextMenu={(hour, x, y) => menu.open(x, y, [jumpItem(runtime, track.id, hour)])} />
    </div>
    {menu.menu}
  </div>;
}
