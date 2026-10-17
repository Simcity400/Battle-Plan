import ArtFallback from '../../components/ArtFallback';
import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { hideTooltip, showTooltip, tooltipStore } from '../../charts/tooltip';
import type { WidgetProps } from '../../dashboard/definitions';
import { useIdeaHistory } from '../../dashboard/ideaHistory';
import { pendingAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { ideaTooltip, tooltipText } from '../../dashboard/ideaTooltip';
import { useIdeaTooltips } from '../../dashboard/useIdeaTooltips';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { OPTIONS } from './definition';
import { inSpans } from '../../charts/timeline/layout';
import { activeAt, spiritState, spirits, startUnseen } from './spiritsData';
import type { Spirit } from './spiritsData';
import './nationalSpirits.css';

const NO_WARNINGS: string[] = [];
/** How long a spirit that ended stays on screen fading out. */
const EXIT_MS = 450;

/** The country's national spirits at the playback moment, as the game's grid of
 * spirit icons. Spirits taken on pop in; spirits that end fade out. */
export default function NationalSpiritsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const widgetOwner = useId();
  const options = resolveOptions(OPTIONS, widget.options);
  const { track, tag, data, error, retry } = useIdeaHistory(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const held = data?.held?.hour === Math.floor(hour ?? NaN) ? data.held : undefined;
  // Each recorded holder starts a new segment, even when it reuses the same tag.
  const identityStart = data?.identityStarts.reduce<number | null>((latest, start) => hour != null && start <= hour ? start : latest, null) ?? null;
  const context = JSON.stringify([track?.source.id, track?.source.revision, track?.source.metadata.campaignKey, track?.branch.id, tag, held ? [held.slot, held.countryId] : identityStart]);
  const owner = `${widgetOwner}:${context}`;
  const menu = useContextMenu('National spirit actions');
  const list = useMemo(() => data ? spirits(held ? { ...data, spans: held.spans } : data) : [], [data, held]);
  const shown = useMemo(() => {
    if (hour == null) return [];
    const items = activeAt(list, hour);
    return held ? items.sort((a, b) => held.spans.findIndex(span => span.token === a.token) - held.spans.findIndex(span => span.token === b.token)) : items;
  }, [list, hour, held]);
  // The game's tooltip for each spirit: its effects formatted as the game does, then its description.
  const tooltips = useIdeaTooltips(data, track, runtime.client, active);
  // The spirit whose tooltip is open, so it is shown again when names arrive.
  const shownTip = useRef<{ context: string; token: string; x: number; y: number } | null>(null);
  const reshow = useRef<() => void>(() => {});
  // Reset each render; only a render that shows the spirits sets it again.
  reshow.current = () => {};
  useEffect(() => {
    // New data or a new country: the tooltip that was open belongs to what came before.
    if (!tooltips || shownTip.current?.context !== context) { shownTip.current = null; reshow.current = () => {}; return; }
    reshow.current();
    // Names arriving, or playback moving to another hour (and perhaps another acquisition).
  }, [tooltips, hour, context]);
  // Spirits that just ended stay briefly so they can fade out. Each has its own
  // timer, which later playback ticks never cancel; unmounting clears them all.
  const [departing, setDeparting] = useState<{ context: string; spirits: Spirit[] }>({ context, spirits: [] });
  const leaving = departing.context === context ? departing.spirits : [];
  const previous = useRef<{ context: string; spirits: Spirit[] }>({ context, spirits: [] });
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  useEffect(() => {
    if (previous.current.context !== context) {
      for (const timer of timers.current.values()) clearTimeout(timer);
      timers.current.clear();
      previous.current = { context, spirits: shown };
      setDeparting({ context, spirits: [] });
      return;
    }
    const gone = previous.current.spirits.filter(spirit => !shown.some(item => item.token === spirit.token));
    previous.current = { context, spirits: shown };
    for (const spirit of gone) {
      if (timers.current.has(spirit.token)) continue;
      setDeparting(current => ({ context, spirits: [...current.spirits, spirit] }));
      const timer = setTimeout(() => {
        if (previous.current.context !== context || timers.current.get(spirit.token) !== timer) return;
        timers.current.delete(spirit.token);
        setDeparting(current => current.context === context ? { context, spirits: current.spirits.filter(item => item.token !== spirit.token) } : current);
      }, EXIT_MS);
      timers.current.set(spirit.token, timer);
    }
  }, [shown, context]);
  useEffect(() => () => { for (const timer of timers.current.values()) clearTimeout(timer); }, []);
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => { onDiagnostics(NO_WARNINGS); hideTooltip(owner); };
  }, [data, onDiagnostics, owner]);

  if (!track || !tag) return <WidgetState kind="empty" icon="star" message="Load a campaign and choose a country to see its national spirits." />;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading national spirits…" />;
  if (widget.time.mode === 'pin' && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (!held && pendingAt(preparedPrefix(data), hour)) return <WidgetState kind="loading" label="This range is still being prepared." />;
  // Unrecorded is not the same as holding no spirits.
  if (inSpans(data.unobserved, hour)) return <WidgetState kind="unavailable" message="National spirits were not recorded at this moment." />;
  // Held ideas content cannot classify might be spirits: then the set is unknown, not empty.
  const unplaced = new Set(data.ideas.filter(info => !info.category).map(info => info.token));
  const unclassified = data.spans.some(span => unplaced.has(span.token) && span.firstHour <= hour && hour <= span.lastHour);
  if (!shown.length && !leaving.length) return unclassified
    ? <WidgetState kind="unavailable" message="National spirits unavailable at this time." />
    : <WidgetState kind="empty" message="No national spirits at this moment." />;

  const since = (spirit: Spirit) => { const state = spiritState(spirit, hour); return state.kind === 'held' ? state.since : null; };
  const describe = (spirit: Spirit) => {
    if (held) return 'Acquisition date still being prepared';
    const start = since(spirit);
    if (start == null) return undefined;
    // Held when recording began (or after an unrecorded stretch): gained at an unknown time.
    return startUnseen(start, track.branch.firstHour, data.unobserved) ? `held before ${dayLabel(start)}` : `since ${dayLabel(start)}`;
  };
  // The hour the spirit held now was taken on: its targets are named as they were then.
  const takenOn = (spirit: Spirit) => held ? undefined : spirit.spans.find(span => span.firstHour <= hour && hour <= span.lastHour)?.firstHour;
  const content = (spirit: Spirit, maxRows?: number) => {
    const held = describe(spirit);
    return tooltips ? ideaTooltip({ title: spirit.name, icon: spirit.icon, subtitle: held ?? undefined },
      tooltips.infos.get(spirit.token), tooltips.sources, tooltips.art.get(spirit.token)?.description, [], undefined, takenOn(spirit), maxRows) : null;
  };
  // The tooltip's effects and description as text, every row, for screen readers.
  const spoken = (spirit: Spirit) => { const tip = content(spirit, Infinity); return tip ? tooltipText(tip) : ''; };
  const tip = (spirit: Spirit, x: number, y: number) => {
    shownTip.current = { context, token: spirit.token, x, y };
    const shown = content(spirit);
    if (shown) showTooltip(owner, shown, x, y, `${spirit.token}:${describe(spirit)}`);
  };
  reshow.current = () => {
    // Only while this widget still shows the tooltip: another chart hovered since keeps it.
    if (tooltipStore.get()?.owner !== owner) return;
    const open = shownTip.current;
    const spirit = open?.context === context && shown.find(item => item.token === open.token);
    if (open && spirit) tip(spirit, open.x, open.y);
    // A spirit that playback ended takes its tooltip with it.
    else if (open) { shownTip.current = null; hideTooltip(owner); }
  };
  const untip = () => { shownTip.current = null; hideTooltip(owner); };
  const tiles = [...shown.map(spirit => ({ spirit, leaving: false })), ...leaving.filter(spirit => !shown.some(item => item.token === spirit.token)).map(spirit => ({ spirit, leaving: true }))];
  return <><ul key={context} className={`national-spirits${options.names === 'under' ? ' with-names' : ''}`} aria-label="National spirits">
    {tiles.map(({ spirit, leaving: out }) => <li key={spirit.token} className={`spirit${out ? ' is-leaving' : ''}`} aria-hidden={out || undefined}
      tabIndex={out ? undefined : 0} aria-label={[spirit.name, describe(spirit)].filter(Boolean).join(', ')} aria-describedby={out ? undefined : `${widgetOwner}-${spirit.token}`}
      onPointerMove={event => tip(spirit, event.clientX, event.clientY)} onPointerLeave={untip}
      onFocus={event => { const box = event.currentTarget.getBoundingClientRect(); tip(spirit, box.right, box.top); }} onBlur={untip}
      onContextMenu={event => {
        if (out) return;
        event.preventDefault();
        const start = takenOn(spirit);
        if (start != null) menu.open(event.clientX, event.clientY, [jumpItem(runtime, track.id, start)]);
      }}>
      {spirit.icon ? <img src={spirit.icon} alt={spirit.name} /> : <ArtFallback className="spirit-missing" icon="sparkle" />}
      <span className="spirit-name">{spirit.name}</span>
      {!out && <span id={`${widgetOwner}-${spirit.token}`} className="sr-only">{spoken(spirit)}</span>}
    </li>)}
  </ul>{held && <p className="chart-line-key" role="status">Acquisition order and context are still being prepared; showing content order.</p>}{menu.menu}</>;
}
