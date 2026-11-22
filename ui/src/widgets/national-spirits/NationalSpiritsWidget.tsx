import ArtFallback from '../../components/ArtFallback';
import { useEffect, useId, useMemo, useRef, useState } from 'react';
import { useContextMenu } from '../../components/ContextMenu';
import WidgetState from '../../components/WidgetState';
import { dayLabel } from '../../charts/draw';
import { jumpItem } from '../../charts/jump';
import { hideTooltip, showTooltip, tooltipStore } from '../../charts/tooltip';
import type { WidgetProps } from '../../dashboard/definitions';
import { useIdeaHistory, useIdeaHour } from '../../dashboard/ideaHistory';
import { ideaTooltip, tooltipText } from '../../dashboard/ideaTooltip';
import { useIdeaTooltips } from '../../dashboard/useIdeaTooltips';
import { trackHourOf } from '../../dashboard/settledHour';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { OPTIONS } from './definition';
import { inSpans } from '../../charts/timeline/layout';
import { activeAt, spiritState, spirits, startUnseen } from './spiritsData';
import type { Spirit } from './spiritsData';
import './nationalSpirits.css';
import { useRegion, useRegionCoverage } from '../../dashboard/widgetTime';

const NO_WARNINGS: string[] = [];
const EXIT_MS = 450;

export default function NationalSpiritsWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const widgetOwner = useId();
  const options = resolveOptions(OPTIONS, widget.options);
  const { track, tag, data, error, retry } = useIdeaHistory(widget, runtime, active);
  const hour = useIdeaHour(widget, runtime, track, data, active);
  const coverage = useRegionCoverage(runtime, useRegion(widget));
  const live = widget.time.mode === 'follow';
  useShownEachFrame(runtime.clock, () => data ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) : null);
  const window = track && widget.time.mode === 'pin' ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null;
  const identityStart = data?.identityStarts.reduce<number | null>((latest, start) => hour != null && start <= hour ? start : latest, null) ?? null;
  const context = JSON.stringify([track?.source.id, track?.source.revision, track?.source.metadata.campaignKey, track?.branch.id, tag, identityStart]);
  const owner = `${widgetOwner}:${context}`;
  const menu = useContextMenu('National spirit actions');
  const list = useMemo(() => data ? spirits(data) : [], [data]);
  const shown = useMemo(() => hour == null ? [] : activeAt(list, hour), [list, hour]);
  const tooltips = useIdeaTooltips(data, track, runtime.client, active);
  const shownTip = useRef<{ context: string; token: string; x: number; y: number } | null>(null);
  const reshow = useRef<() => void>(() => {});
  reshow.current = () => {};
  useEffect(() => {
    if (!tooltips || shownTip.current?.context !== context) { shownTip.current = null; reshow.current = () => {}; return; }
    reshow.current();
  }, [tooltips, hour, context]);
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
  const failure = error ? <WidgetState kind="error" failure={error} onRetry={retry} /> : null;
  if (widget.time.mode === 'pin' && !window) return <WidgetState kind="unavailable" message="Choose another time in widget settings." />;
  if (coverage.outside || !live && hour == null) return <WidgetState kind="outside" read={coverage} />;
  if (failure && !data) return failure;
  if (!data) return <>{failure}<WidgetState kind="loading" label="Loading national spirits…" /></>;
  if (hour == null) return <>{failure}<WidgetState kind="loading" label="Loading national spirits…" /></>;
  if (inSpans(data.unobserved, hour)) return <>{failure}<WidgetState kind="unavailable" message="National spirits were not recorded at this moment." /></>;
  const unplaced = new Set(data.ideas.filter(info => !info.category).map(info => info.token));
  const unclassified = data.spans.some(span => unplaced.has(span.token) && span.firstHour <= hour && hour <= span.lastHour);
  if (!shown.length && !leaving.length) return <>{failure}{unclassified
    ? <WidgetState kind="unavailable" message="National spirits unavailable at this time." />
    : <WidgetState kind="empty" message="No national spirits at this moment." />}</>;

  const since = (spirit: Spirit) => { const state = spiritState(spirit, hour); return state.kind === 'held' ? state.since : null; };
  const describe = (spirit: Spirit) => {
    const start = since(spirit);
    if (start == null) return undefined;
    return startUnseen(start, track.branch.firstHour, data.unobserved) ? `held before ${dayLabel(start)}` : `since ${dayLabel(start)}`;
  };
  const takenOn = (spirit: Spirit) => spirit.spans.find(span => span.firstHour <= hour && hour <= span.lastHour)?.firstHour;
  const content = (spirit: Spirit, maxRows?: number) => {
    const subtitle = describe(spirit);
    return tooltips ? ideaTooltip({ title: spirit.name, icon: spirit.icon, subtitle },
      tooltips.infos.get(spirit.token), tooltips.sources, tooltips.art.get(spirit.token)?.description, [], spirit.nameMissing ?? undefined, takenOn(spirit), maxRows) : null;
  };
  const spoken = (spirit: Spirit) => { const tip = content(spirit, Infinity); return tip ? tooltipText(tip) : ''; };
  const tip = (spirit: Spirit, x: number, y: number) => {
    shownTip.current = { context, token: spirit.token, x, y };
    const shown = content(spirit);
    if (shown) showTooltip(owner, shown, x, y, `${spirit.token}:${describe(spirit)}`);
  };
  reshow.current = () => {
    if (tooltipStore.get()?.owner !== owner) return;
    const open = shownTip.current;
    const spirit = open?.context === context && shown.find(item => item.token === open.token);
    if (open && spirit) tip(spirit, open.x, open.y);
    else if (open) { shownTip.current = null; hideTooltip(owner); }
  };
  const untip = () => { shownTip.current = null; hideTooltip(owner); };
  const tiles = [...shown.map(spirit => ({ spirit, leaving: false })), ...leaving.filter(spirit => !shown.some(item => item.token === spirit.token)).map(spirit => ({ spirit, leaving: true }))];
  return <>{failure}<ul key={context} className={`national-spirits${options.names === 'under' ? ' with-names' : ''}`} aria-label="National spirits">
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
  </ul>{menu.menu}</>;
}
