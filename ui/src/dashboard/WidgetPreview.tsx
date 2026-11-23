import { useCallback, useContext, useMemo, useRef, useState } from 'react';
import type { ReactNode, SyntheticEvent } from 'react';
import { Icon } from '../components/Icons';
import { StyledIcon } from '../shell/appearance/StyledIcon';
import { WidgetIconsContext } from '../shell/appearance/WidgetIconMenu';
import type { WidgetDefinition, WidgetRuntime, WidgetSkeletonKind } from './definitions';
import { createWidget, recordingEndTime } from './model';
import type { WidgetSpec } from './model';
import { resolveOptions } from './options';
import { WidgetBody } from './widgets';
import { HeldFailures, HeldFailuresContext } from './heldFailures';
import HeldFailureNotices from './HeldFailureNotices';
import DefaultCountryChip from './CountryChip';
import { timeChipSummary } from './WidgetTimeChip';
import { WidgetFrameContext, widgetWording } from './widgetFrame';
import type { WidgetFrameValue } from './widgetFrame';

const ROW = 26, GAP = 6;
const HEIGHT = { min: 240, max: 440 };

export default function WidgetPreview({ definition, initial, runtime }: {
  definition: WidgetDefinition; initial?: Pick<WidgetSpec, 'title' | 'options'>; runtime: WidgetRuntime;
}) {
  const trackId = runtime.campaignTracks[0]?.id;
  const widget = useMemo(() => {
    if (!trackId) return null;
    const widget = { ...createWidget(definition.kind, trackId, 'catalogue-preview'), ...initial };
    return { ...widget, time: recordingEndTime(definition, runtime.tracks) ?? widget.time };
  }, [definition, initial, trackId, runtime.tracks]);
  const icon = useContext(WidgetIconsContext).style(definition.kind);
  const [summary, setSummary] = useState<HTMLSpanElement | null>(null);
  const [country, setCountry] = useState<HTMLSpanElement | null>(null);
  const [captionSlot, setCaptionSlot] = useState<HTMLDivElement | null>(null);
  const [hasBodyState, setHasBodyState] = useState(false);
  const stateOwners = useRef(new Set<object>());
  const [failures] = useState(() => new HeldFailures());
  const reportState = useCallback((owner: object, active: boolean) => {
    if (active) stateOwners.current.add(owner); else stateOwners.current.delete(owner);
    setHasBodyState(stateOwners.current.size > 0);
  }, []);
  const noop = useCallback(() => {}, []);
  const wording = useMemo(() => widgetWording(definition, widget?.title), [definition, widget?.title]);
  const track = runtime.tracks.find(item => item.id === trackId);
  const frameValue = useMemo<WidgetFrameValue>(() => ({
    title: widget?.title ?? definition.title,
    subject: wording.subject,
    things: wording.things,
    timeMode: widget?.time.mode ?? definition.defaultTime,
    pinned: widget?.time.mode === 'pin' || widget?.time.mode === 'range',
    bounds: track ? { firstHour: track.branch.firstHour, lastHour: track.branch.lastHour } : null,
    hasSubject: definition.data !== false && definition.countrySelection !== false,
    captionSlot,
    reportState,
    announce: noop,
    setDynamicViewSwitch: noop,
    skeleton: (definition.skeleton ?? 'generic') as WidgetSkeletonKind | (() => ReactNode),
    setSkeleton: noop,
  }), [widget, definition, wording, track, captionSlot, reportState, noop]);

  if (definition.preview === false || !widget || !runtime.tracks.length) return <div className="widget-preview is-empty">
    <StyledIcon icon={icon.icon} fallback={definition.icon} colour={icon.colour} /><span>{definition.preview === false ? 'This widget draws on the page itself, so it has no preview here.' : 'Load a campaign to preview widgets with its data.'}</span></div>;
  const height = Math.min(Math.max(definition.defaultSize.h * (ROW + GAP) - GAP, HEIGHT.min), HEIGHT.max);
  const stop = (event: SyntheticEvent) => { event.preventDefault(); event.stopPropagation(); };
  const CountryChip = definition.countryChip ?? DefaultCountryChip;
  const hasSubject = definition.data !== false && definition.countrySelection !== false;
  const timeInfo = definition.data !== false && definition.timeModes.length > 0
    ? timeChipSummary(widget, widget, track, runtime.tracks, runtime.clock.getHour?.() ?? 0) : null;
  const viewSwitchLabel = (() => {
    const spec = definition.viewSwitch;
    if (!spec || !definition.options) return null;
    const resolved = resolveOptions(definition.options, widget.options);
    if (spec.when && !spec.when(resolved)) return null;
    const field = definition.options.find(item => item.key === spec.option);
    if (!field || field.kind !== 'choice') return null;
    const options = spec.choices ?? field.choices;
    const current = options.find(item => item.value === resolved[spec.option]) ?? options[0];
    return current?.label ?? null;
  })();

  return <div className="widget-preview" style={{ height }} onClickCapture={stop} onDoubleClickCapture={stop} onContextMenuCapture={stop}
    onKeyDownCapture={event => { if (event.key === 'Enter' || event.key === ' ') stop(event); }}>
    <article className={`dash-widget kind-${definition.kind}`} aria-label={`${widget.title} preview`}>
      <header className="dash-widget-header">
        <span className="dash-widget-icon" aria-hidden="true" style={icon.colour ? { color: icon.colour } : undefined}><StyledIcon icon={icon.icon} fallback={definition.icon} /></span>
        <div className="dash-widget-title"><h2>{widget.title}</h2></div>
        <span className="dash-widget-summary" ref={setSummary} hidden={hasBodyState || undefined} />
        <span className="dash-header-grow" aria-hidden="true" />
        <div className="dash-widget-controls">
          {hasSubject && <CountryChip widget={widget} runtime={runtime} active={false} onSlot={setCountry} />}
          {viewSwitchLabel && <span className="dash-view-menu-trigger is-static">{viewSwitchLabel}</span>}
          {timeInfo && <span className="dash-time-chip is-static" data-time-mode={timeInfo.mode}>
            {timeInfo.mode === 'follow' ? <span className="dash-time-dot" aria-hidden="true" />
              : timeInfo.mode === 'pin' ? <span className="dash-time-dot is-pinned" aria-hidden="true" />
              : <Icon name="clock" className="dash-time-icon" />}
            <span className="dash-time-label">{timeInfo.label}</span>
          </span>}
        </div>
      </header>
      <WidgetFrameContext.Provider value={frameValue}>
        <HeldFailureNotices failures={failures} />
        <div className="dash-widget-caption-slot" ref={setCaptionSlot} hidden={hasBodyState || undefined} />
        <div className="dash-widget-content"><div className="dash-widget-scaled">
          <HeldFailuresContext.Provider value={failures}>
            <WidgetBody key={`${definition.kind}:${JSON.stringify(initial?.options ?? {})}`} widget={widget} runtime={runtime} headerSlot={summary} countrySlot={country} />
          </HeldFailuresContext.Provider>
        </div></div>
      </WidgetFrameContext.Provider>
    </article>
  </div>;
}
