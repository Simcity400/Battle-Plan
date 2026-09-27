import { useEffect, useId, useMemo, useRef, useState } from 'react';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { hideTooltip, showTooltip, tooltipStore } from '../../charts/tooltip';
import type { WidgetProps } from '../../dashboard/definitions';
import { useIdeaHistory } from '../../dashboard/ideaHistory';
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
  const owner = useId();
  const options = resolveOptions(OPTIONS, widget.options);
  const { track, tag, data, error, retry } = useIdeaHistory(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const list = useMemo(() => data ? spirits(data) : [], [data]);
  const shown = useMemo(() => hour == null ? [] : activeAt(list, hour), [list, hour]);
  // The game's tooltip for each spirit: its effects formatted as the game does, then its description.
  const tooltips = useIdeaTooltips(data, track, runtime.client, active);
  // The spirit whose tooltip is open, so it is shown again when names arrive.
  const shownTip = useRef<{ token: string; x: number; y: number } | null>(null);
  const reshow = useRef<() => void>(() => {});
  // Reset each render; only a render that shows the spirits sets it again.
  reshow.current = () => {};
  useEffect(() => {
    // New data or a new country: the tooltip that was open belongs to what came before.
    if (!tooltips) { shownTip.current = null; reshow.current = () => {}; return; }
    reshow.current();
    // Names arriving, or playback moving to another hour (and perhaps another acquisition).
  }, [tooltips, hour]);
  // Spirits that just ended stay briefly so they can fade out. Each has its own
  // timer, which later playback ticks never cancel; unmounting clears them all.
  const [leaving, setLeaving] = useState<Spirit[]>([]);
  const previous = useRef<Spirit[]>([]);
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  useEffect(() => {
    const gone = previous.current.filter(spirit => !shown.some(item => item.token === spirit.token));
    previous.current = shown;
    for (const spirit of gone) {
      if (timers.current.has(spirit.token)) continue;
      setLeaving(current => [...current, spirit]);
      timers.current.set(spirit.token, setTimeout(() => {
        timers.current.delete(spirit.token);
        setLeaving(current => current.filter(item => item.token !== spirit.token));
      }, EXIT_MS));
    }
  }, [shown]);
  useEffect(() => () => { for (const timer of timers.current.values()) clearTimeout(timer); }, []);
  useEffect(() => {
    onDiagnostics(data?.warnings.length ? data.warnings : NO_WARNINGS);
    return () => { onDiagnostics(NO_WARNINGS); hideTooltip(owner); };
  }, [data, onDiagnostics, owner]);

  if (!track || !tag) return <WidgetState kind="empty" icon="star" message="Load a campaign and choose a country to see its national spirits." />;
  if (error) return <WidgetState kind="error" message={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading national spirits…" />;
  if (widget.time.mode === 'pin' && !window) return <WidgetState kind="unavailable" message="The chosen moment is not in this campaign. Choose it again in the widget settings." />;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <WidgetState kind="unavailable" message="This time is outside the country’s recorded branch." />;
  // Unrecorded is not the same as holding no spirits.
  if (inSpans(data.unobserved, hour)) return <WidgetState kind="unavailable" message="National spirits were not recorded at this moment." />;
  // Held ideas content cannot classify might be spirits: then the set is unknown, not empty.
  const unplaced = new Set(data.ideas.filter(info => !info.category).map(info => info.token));
  const unclassified = data.spans.some(span => unplaced.has(span.token) && span.firstHour <= hour && hour <= span.lastHour);
  if (!shown.length && !leaving.length) return unclassified
    ? <WidgetState kind="unavailable" message="Some ideas held at this moment could not be classified from the content, so the national spirits are not known." />
    : <WidgetState kind="empty" message="No national spirits at this moment." />;

  const since = (spirit: Spirit) => { const state = spiritState(spirit, hour); return state.kind === 'held' ? state.since : null; };
  const describe = (spirit: Spirit) => {
    const start = since(spirit);
    if (start == null) return undefined;
    // Held when recording began (or after an unrecorded stretch): gained at an unknown time.
    return startUnseen(start, track.branch.firstHour, data.unobserved) ? `held before ${dayLabel(start)}` : `since ${dayLabel(start)}`;
  };
  // The hour the spirit held now was taken on: its targets are named as they were then.
  const takenOn = (spirit: Spirit) => spirit.spans.find(span => span.firstHour <= hour && hour <= span.lastHour)?.firstHour;
  const content = (spirit: Spirit, maxRows?: number) => {
    const held = describe(spirit);
    return tooltips ? ideaTooltip({ title: spirit.name, icon: spirit.icon, subtitle: held ? `National spirit · ${held}` : 'National spirit' },
      tooltips.infos.get(spirit.token), tooltips.sources, tooltips.art.get(spirit.token)?.description, [], undefined, takenOn(spirit), maxRows) : null;
  };
  // The tooltip's effects and description as text, every row, for screen readers.
  const spoken = (spirit: Spirit) => { const tip = content(spirit, Infinity); return tip ? tooltipText(tip) : ''; };
  const tip = (spirit: Spirit, x: number, y: number) => {
    shownTip.current = { token: spirit.token, x, y };
    const shown = content(spirit);
    if (shown) showTooltip(owner, shown, x, y, `${spirit.token}:${describe(spirit)}`);
  };
  reshow.current = () => {
    // Only while this widget still shows the tooltip: another chart hovered since keeps it.
    if (tooltipStore.get()?.owner !== owner) return;
    const open = shownTip.current;
    const spirit = open && shown.find(item => item.token === open.token);
    if (open && spirit) tip(spirit, open.x, open.y);
    // A spirit that playback ended takes its tooltip with it.
    else if (open) { shownTip.current = null; hideTooltip(owner); }
  };
  const untip = () => { shownTip.current = null; hideTooltip(owner); };
  const tiles = [...shown.map(spirit => ({ spirit, leaving: false })), ...leaving.filter(spirit => !shown.some(item => item.token === spirit.token)).map(spirit => ({ spirit, leaving: true }))];
  return <ul className={`national-spirits${options.names === 'under' ? ' with-names' : ''}`} aria-label="National spirits">
    {tiles.map(({ spirit, leaving: out }) => <li key={spirit.token} className={`spirit${out ? ' is-leaving' : ''}`} aria-hidden={out || undefined}
      tabIndex={out ? undefined : 0} aria-label={[spirit.name, describe(spirit)].filter(Boolean).join(', ')} aria-describedby={out ? undefined : `${owner}-${spirit.token}`}
      onPointerMove={event => tip(spirit, event.clientX, event.clientY)} onPointerLeave={untip}
      onFocus={event => { const box = event.currentTarget.getBoundingClientRect(); tip(spirit, box.right, box.top); }} onBlur={untip}>
      {spirit.icon ? <img src={spirit.icon} alt={spirit.name} /> : <span className="spirit-missing" aria-hidden="true">✦</span>}
      <span className="spirit-name">{spirit.name}</span>
      {!out && <span id={`${owner}-${spirit.token}`} className="sr-only">{spoken(spirit)}</span>}
    </li>)}
  </ul>;
}
