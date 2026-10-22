import { useEffect, useMemo, useRef, useState } from 'react';
import type { ComponentType } from 'react';
import WidgetState from '../components/WidgetState';
import { CountryFlagUrl } from '../components/CountryFlag';
import { useCountryLabelsAt } from './countryFlagsAt';
import { useComparisonCountries } from './comparisonCountries';
import { MEMBERSHIP_PREPARING } from './groups';
import type { WidgetProps } from './definitions';
import { resolveWindow } from './model';
import { usePlaybackHour } from './playback';
import { useDiagnostics } from './useDiagnostics';
import type { HourSpan } from '../generated/dashboard';
import './countryComparisons.css';

/** Independent country panels share settings and keep their exact country data.
 * No aggregate is offered for balances with country-specific denominators. */
export type ComparisonProps = WidgetProps & { within?: readonly HourSpan[] | null };
const NO_WARNINGS: string[] = [];

export default function CountryComparisons({ component: Component, ...props }: WidgetProps & { component: ComponentType<ComparisonProps> }) {
  const { widget, runtime, active, onDiagnostics } = props;
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const { track, countries, error, refreshError, retry, pending, contentPending } = useComparisonCountries(widget, runtime, active);
  const hour = track ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour())?.lastHour ?? track.branch.lastHour : 0;
  const [notes, setNotes] = useState<ReadonlyMap<string, readonly string[]>>(new Map());
  const body = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ left: 0, width: 320 });
  useEffect(() => {
    const element = body.current;
    if (!element) return;
    const update = () => setView({ left: element.scrollLeft, width: element.clientWidth });
    const observer = new ResizeObserver(update);
    observer.observe(element);
    update();
    return () => observer.disconnect();
  }, [pending, error, countries.reason]);
  const multiple = countries.tags.length > 1;
  const panelWidth = Math.max(320, view.width / Math.min(2, countries.tags.length || 1));
  const first = multiple ? Math.min(Math.max(0, countries.tags.length - 1), Math.max(0, Math.floor(view.left / panelWidth) - 1)) : 0;
  const last = multiple ? Math.min(countries.tags.length, Math.ceil((view.left + view.width) / panelWidth) + 1) : countries.tags.length;
  const shown = useMemo(() => countries.tags.slice(first, Math.max(first + 1, last)), [countries.tags, first, last]);
  const wanted = useMemo(() => shown.map(tag => ({ hour, tag })), [shown, hour]);
  const labels = useCountryLabelsAt(runtime.client, track, wanted, active);
  useEffect(() => {
    if (body.current) body.current.scrollLeft = 0;
    setView(before => ({ ...before, left: 0 }));
    setNotes(new Map());
  }, [track?.source.id, track?.source.revision, track?.branch.id, countries.tags]);
  // A panel resending an equal list keeps the notes, so it cannot re-render every panel again.
  const handlers = useMemo(() => new Map(countries.tags.map(tag => [tag, (warnings: readonly string[]) => setNotes(before => {
    const held = before.get(tag) ?? NO_WARNINGS;
    if (held.length === warnings.length && held.every((note, index) => note === warnings[index])) return before;
    return new Map(before).set(tag, warnings);
  })])), [countries.tags]);
  const diagnostics = useMemo(() => [...(contentPending ? ['Group names are updating from the recorded content.'] : []),
    ...new Set(countries.tags.flatMap(tag => (notes.get(tag) ?? []).map(note => `${tag}: ${note}`)))], [countries.tags, notes, contentPending]);
  useDiagnostics(onDiagnostics, diagnostics);
  // Each mounted panel keeps its widget while this one re-renders, so its own hooks do not re-run.
  const panels = useMemo(() => new Map(shown.map(tag => [tag, { ...widget, countries: { kind: 'tags' as const, tags: [tag] } }])), [widget, shown]);
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  const refreshNotice = refreshError && <WidgetState kind="error" detail={refreshError} onRetry={retry} />;
  if (pending) return <WidgetState kind="loading" label="Loading countries…" />;
  if (countries.reason) return <>{refreshNotice}<WidgetState kind="empty" message={countries.reason} /></>;
  // A group's members at hours still being prepared are not known yet: no country is not proof of none.
  if (!countries.tags.length && track && !countries.known(track.branch.firstHour, track.branch.lastHour)) return <>{refreshNotice}<WidgetState kind="loading" label={MEMBERSHIP_PREPARING} /></>;
  return <>{refreshNotice}<div ref={body} className="country-comparisons" data-multiple={multiple || undefined}
    onScroll={event => setView({ left: event.currentTarget.scrollLeft, width: event.currentTarget.clientWidth })}>
    {first > 0 && <div aria-hidden="true" style={{ flex: `0 0 ${first * panelWidth}px` }} />}
    {shown.map(tag => {
      const label = labels.labelAt(hour)(tag);
      return <section key={tag} className="country-comparison" aria-label={label.name} style={multiple ? { flex: `0 0 ${panelWidth}px` } : undefined}>
        {countries.tags.length > 1 && <p className="country-comparison-name"><CountryFlagUrl className="country-comparison-flag" url={label.flag} reason={label.flagReason} pending={label.flagPending} />{label.name}</p>}
        <Component {...props} within={countries.membership?.get(tag)} onDiagnostics={handlers.get(tag)!}
          widget={panels.get(tag)!} />
      </section>;
    })}
    {last < countries.tags.length && <div aria-hidden="true" style={{ flex: `0 0 ${(countries.tags.length - last) * panelWidth}px` }} />}
  </div></>;
}
