import { useMemo, useState } from 'react';
import type { SyntheticEvent } from 'react';
import { Icon } from '../components/Icons';
import type { WidgetDefinition, WidgetRuntime } from './definitions';
import { createWidget, recordingEndTime } from './model';
import type { WidgetSpec } from './model';
import { WidgetBody } from './widgets';

// The grid's row geometry, so a preview is as tall as the widget sits on the page.
const ROW = 26, GAP = 6;
/** Preview heights in CSS pixels: the widget's default height, kept to a range the details pane can show. */
const HEIGHT = { min: 240, max: 440 };

/** The real widget, drawn from the loaded campaign at the details pane's width and its own height.
 * It is never scaled, so text and charts stay sharp; widgets lay out for narrow columns already.
 * It scrolls and shows its hover details, but clicks stop at its edge, so its controls and
 * pop-ups cannot open over (or close) the catalogue. */
export default function WidgetPreview({ definition, initial, runtime }: {
  definition: WidgetDefinition; initial?: Pick<WidgetSpec, 'title' | 'options'>; runtime: WidgetRuntime;
}) {
  const trackId = runtime.campaignTracks[0]?.id;
  // At the recording's end, not wherever playback stands: the opening hours often have no data yet.
  const widget = useMemo(() => {
    if (!trackId) return null;
    const widget = { ...createWidget(definition.kind, trackId, 'catalogue-preview'), ...initial };
    return { ...widget, time: recordingEndTime(definition, runtime.tracks) ?? widget.time };
  }, [definition, initial, trackId, runtime.tracks]);
  const [summary, setSummary] = useState<HTMLSpanElement | null>(null);
  const [country, setCountry] = useState<HTMLSpanElement | null>(null);
  if (definition.preview === false || !widget || !runtime.tracks.length) return <div className="widget-preview is-empty">
    <Icon name={definition.icon} /><span>{definition.preview === false ? 'This widget draws on the page itself, so it has no preview here.' : 'Load a campaign to preview widgets with its data.'}</span></div>;
  const height = Math.min(Math.max(definition.defaultSize.h * (ROW + GAP) - GAP, HEIGHT.min), HEIGHT.max);
  const stop = (event: SyntheticEvent) => { event.preventDefault(); event.stopPropagation(); };
  return <div className="widget-preview" style={{ height }} onClickCapture={stop} onDoubleClickCapture={stop} onContextMenuCapture={stop}
    onKeyDownCapture={event => { if (event.key === 'Enter' || event.key === ' ') stop(event); }}>
    <article className={`dash-widget kind-${definition.kind}`} aria-label={`${widget.title} preview`}>
      <header className="dash-widget-header">
        <span className="dash-widget-icon" aria-hidden="true"><Icon name={definition.icon} /></span>
        <div className="dash-widget-title"><h2>{widget.title}<span className="dash-widget-summary" ref={setSummary} /></h2></div>
        <span ref={setCountry} />
      </header>
      <div className="dash-widget-content"><div className="dash-widget-scaled">
        <WidgetBody key={`${definition.kind}:${JSON.stringify(initial?.options ?? {})}`} widget={widget} runtime={runtime} headerSlot={summary} countrySlot={country} />
      </div></div>
    </article>
  </div>;
}
