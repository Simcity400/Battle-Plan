import { useContext, useMemo, useState } from 'react';
import type { SyntheticEvent } from 'react';
import { StyledIcon } from '../shell/appearance/StyledIcon';
import { WidgetIconsContext } from '../shell/appearance/WidgetIconMenu';
import type { WidgetDefinition, WidgetRuntime } from './definitions';
import { createWidget, recordingEndTime } from './model';
import type { WidgetSpec } from './model';
import { WidgetBody } from './widgets';
import { HeldFailures, HeldFailuresContext } from './heldFailures';
import HeldFailureNotices from './HeldFailureNotices';

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
  const [failures] = useState(() => new HeldFailures());
  if (definition.preview === false || !widget || !runtime.tracks.length) return <div className="widget-preview is-empty">
    <StyledIcon icon={icon.icon} fallback={definition.icon} colour={icon.colour} /><span>{definition.preview === false ? 'This widget draws on the page itself, so it has no preview here.' : 'Load a campaign to preview widgets with its data.'}</span></div>;
  const height = Math.min(Math.max(definition.defaultSize.h * (ROW + GAP) - GAP, HEIGHT.min), HEIGHT.max);
  const stop = (event: SyntheticEvent) => { event.preventDefault(); event.stopPropagation(); };
  return <div className="widget-preview" style={{ height }} onClickCapture={stop} onDoubleClickCapture={stop} onContextMenuCapture={stop}
    onKeyDownCapture={event => { if (event.key === 'Enter' || event.key === ' ') stop(event); }}>
    <article className={`dash-widget kind-${definition.kind}`} aria-label={`${widget.title} preview`}>
      <header className="dash-widget-header">
        <span className="dash-widget-icon" aria-hidden="true" style={icon.colour ? { color: icon.colour } : undefined}><StyledIcon icon={icon.icon} fallback={definition.icon} /></span>
        <div className="dash-widget-title"><h2>{widget.title}<span className="dash-widget-summary" ref={setSummary} /></h2></div>
        <span ref={setCountry} />
      </header>
      <HeldFailureNotices failures={failures} />
      <div className="dash-widget-content"><div className="dash-widget-scaled">
        <HeldFailuresContext.Provider value={failures}>
          <WidgetBody key={`${definition.kind}:${JSON.stringify(initial?.options ?? {})}`} widget={widget} runtime={runtime} headerSlot={summary} countrySlot={country} />
        </HeldFailuresContext.Provider>
      </div></div>
    </article>
  </div>;
}
