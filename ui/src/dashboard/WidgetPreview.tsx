import { useCallback, useContext, useMemo, useRef, useState } from 'react';
import type { ReactNode, SyntheticEvent } from 'react';
import { StyledIcon } from '../shell/appearance/StyledIcon';
import { WidgetIconsContext } from '../shell/appearance/WidgetIconMenu';
import type { WidgetDefinition, WidgetRuntime, WidgetSkeletonKind } from './definitions';
import { createWidget, recordingEndTime } from './model';
import type { WidgetSpec } from './model';
import { WidgetBody } from './widgets';
import { HeldFailures, HeldFailuresContext } from './heldFailures';
import HeldFailureNotices from './HeldFailureNotices';
import WidgetHeader from './WidgetHeader';
import { resolveDeclaredViewSwitch, WidgetFrameContext, widgetWording } from './widgetFrame';
import type { DynamicViewSwitch, WidgetFrameValue } from './widgetFrame';

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
  const [dynamicSwitch, setDynamicSwitch] = useState<DynamicViewSwitch | null>(null);
  const switchOwners = useRef(new Map<object, DynamicViewSwitch>());
  const [failures] = useState(() => new HeldFailures());
  const reportState = useCallback((owner: object, active: boolean) => {
    if (active) stateOwners.current.add(owner); else stateOwners.current.delete(owner);
    setHasBodyState(stateOwners.current.size > 0);
  }, []);
  const setDynamicViewSwitch = useCallback((owner: object, spec: DynamicViewSwitch | null) => {
    if (spec) switchOwners.current.set(owner, spec); else switchOwners.current.delete(owner);
    setDynamicSwitch([...switchOwners.current.values()].at(-1) ?? null);
  }, []);
  const noop = useCallback(() => {}, []);
  const wording = useMemo(() => widgetWording(definition, widget?.title), [definition, widget?.title]);
  const track = runtime.tracks.find(item => item.id === trackId);
  const declaredSwitch = useMemo(() => widget ? resolveDeclaredViewSwitch(definition, widget, noop) : null, [definition, widget, noop]);
  const activeSwitch = dynamicSwitch ?? declaredSwitch;
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
    clearAnnouncement: noop,
    setDynamicViewSwitch,
    skeleton: (definition.skeleton ?? 'generic') as WidgetSkeletonKind | (() => ReactNode),
    setSkeleton: noop,
  }), [widget, definition, wording, track, captionSlot, reportState, noop, setDynamicViewSwitch]);

  if (definition.preview === false || !widget || !runtime.tracks.length) return <div className="widget-preview is-empty">
    <StyledIcon icon={icon.icon} fallback={definition.icon} colour={icon.colour} /><span>{definition.preview === false ? 'This widget draws on the page itself, so it has no preview here.' : 'Load a campaign to preview widgets with its data.'}</span></div>;
  const height = Math.min(Math.max(definition.defaultSize.h * (ROW + GAP) - GAP, HEIGHT.min), HEIGHT.max);
  const stop = (event: SyntheticEvent) => { event.preventDefault(); event.stopPropagation(); };

  return <div className="widget-preview" style={{ height }} onClickCapture={stop} onDoubleClickCapture={stop} onContextMenuCapture={stop}
    onKeyDownCapture={event => { if (event.key === 'Enter' || event.key === ' ') stop(event); }}>
    <article className={`dash-widget kind-${definition.kind}`} aria-label={`${widget.title} preview`}>
      <WidgetHeader
        widget={widget}
        definition={definition}
        effective={widget}
        campaignTrack={track}
        runtime={runtime}
        readOnly
        activeSwitch={activeSwitch}
        hasBodyState={hasBodyState}
        summarySlotRef={setSummary}
        countrySlotRef={setCountry}
        headerSlotElement={summary}
      />
      <WidgetFrameContext.Provider value={frameValue}>
        <HeldFailureNotices failures={failures} />
        <div className="dash-widget-caption-slot" ref={setCaptionSlot} hidden={hasBodyState || undefined} />
        <div className="dash-widget-content"><div className="dash-widget-scaled">
          <HeldFailuresContext.Provider value={failures}>
            <WidgetBody key={`${definition.kind}:${JSON.stringify(initial?.options ?? {})}`} widget={widget} runtime={runtime} headerSlot={summary} countrySlot={country} setOption={noop} setOptions={noop} />
          </HeldFailuresContext.Provider>
        </div></div>
      </WidgetFrameContext.Provider>
    </article>
  </div>;
}
