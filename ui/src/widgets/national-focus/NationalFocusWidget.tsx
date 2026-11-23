import { countryFromKey } from '../../dashboard/countryKey.ts';
import { failureText, failureDetails, failureKey, type Failure } from '../../dashboard/queryError.ts';
import { VirtualList } from '../../components/VirtualList';
import ArtFallback from '../../components/ArtFallback';
import { memo, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { formatDate, gameDate } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { defaultCountryKey } from '../../dashboard/countrySelection';
import { useGate } from '../../dashboard/regionGate';
import GateState from '../../dashboard/GateState';
import { axisAt, SPEC } from '../../dashboard/regionTime';
import { useRegionHourAt } from '../../dashboard/widgetTime';
import { useHeadHour } from '../army-size/headHour';
import { setAttr, setHidden, setText } from '../../charts/domText';
import { useProduct } from '../../dashboard/products';
import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles';
import { useContentRevision } from '../../dashboard/contentRevision';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import type { Artwork, FocusDatabase, FocusEntry, FocusOrder, FocusReply } from '../../generated/dashboard';
import { nameKey, isFailedFact } from '../../dashboard/countryFacts.ts';
import type { NameAt } from '../../dashboard/countryFacts';
import { useContinuousFocusArtwork, useFocusArtwork } from './useFocusArtwork';
import { activeFocus, continuousArtAsks, focusAheadUntil, focusArtAsks, focusAsksAhead, focusDays, focusProgressAt, focusProgressView, focusReadHours, focusRequest, focusShownAt, validateFocusReply } from './focusData';
import type { FocusProgressState, SegmentHours } from './focusData';
import { AHEAD_SECONDS } from '../../charts/reveal';
import WidgetState from '../../components/WidgetState';
import { Icon } from '../../components/Icons';
import { useContextMenu } from '../../components/ContextMenu';
import { rowJumpProps } from '../../charts/jump';
import './nationalFocus.css';
import { progressFill, progressKnob, reducedMotion } from '../../engine/glide.ts';

const NO_WARNINGS: string[] = [];
const sameAsks = (held: NameAt[], next: NameAt[]) => held.length === next.length && held.every((ask, at) => ask.tag === next[at].tag && ask.hour === next[at].hour) ? held : next;
export type FocusListItem = { kind: 'completed'; entry: FocusEntry } | { kind: 'unfinished'; token: string };
export const isCompletedFocus = (item: FocusListItem) => item.kind === 'completed';
const disclosureKey = (entry: FocusEntry, readHour: number) => JSON.stringify([entry.ordinal, nameKey(readHour, entry.token)]);

export interface FocusLiveProgress { at(): FocusProgressState | null; subscribe(changed: () => void): () => void }

export function FocusArt({ art, size }: { art: Artwork | undefined; size: 'large' | 'small' }) {
  return <span className={`focus-art is-${size}`}>{art?.icon ? <img src={art.icon} alt="" /> : <ArtFallback icon="target" />}</span>;
}

function focusErrors(failure: Failure | undefined, failures: readonly Failure[] = []): Failure[] {
  return [...new Map([...failures, ...(failure ? [failure] : [])].map(error => [failureKey(error), error])).values()];
}

export function FocusFailureNotices({ errors }: { errors: readonly Failure[] }) {
  return errors.map(error => <span key={failureKey(error)} data-detail={error.message}>{failureText(error, 'Couldn’t load focus details.')}</span>);
}

interface CompletionProps {
  entry: FocusEntry; art?: Artwork; reason?: string; failure?: Failure; failures?: readonly Failure[]; titleFailure?: Failure; pending?: boolean; description?: string | null;
  open: boolean; onToggle: (open: boolean) => void;
}

const failuresKey = (failures: readonly Failure[] | undefined) => failures?.map(failureKey).join('\u0000') ?? '';
const sameArt = (a: Artwork | undefined, b: Artwork | undefined) => a === b || !!a && !!b && a.token === b.token && a.name === b.name
  && a.icon === b.icon && a.description === b.description && a.baseDurationDays === b.baseDurationDays;

function sameCompletion(a: CompletionProps, b: CompletionProps): boolean {
  return a.entry.ordinal === b.entry.ordinal && a.entry.token === b.entry.token && a.entry.completedByHour === b.entry.completedByHour
    && sameArt(a.art, b.art) && a.reason === b.reason && a.pending === b.pending && a.description === b.description && a.open === b.open
    && (a.failure ? failureKey(a.failure) : '') === (b.failure ? failureKey(b.failure) : '') && (a.titleFailure ? failureKey(a.titleFailure) : '') === (b.titleFailure ? failureKey(b.titleFailure) : '')
    && failuresKey(a.failures) === failuresKey(b.failures);
}

export const FocusCompletion = memo(function FocusCompletion({ entry, art, reason, failure, failures, titleFailure, pending, description, open, onToggle }: CompletionProps) {
  const errors = focusErrors(failure, failures);
  const date = entry.completedByHour == null ? null : gameDate(entry.completedByHour);
  return <details data-detail={failureDetails(errors)} className="focus-details" open={open} onToggle={event => onToggle(event.currentTarget.open)}>
    <summary className="focus-row">
      <span className="focus-number" aria-label={`Focus ${entry.ordinal + 1}`}>{String(entry.ordinal + 1).padStart(2, '0')}</span>
      <FocusArt art={art} size="small" />
      <span className="focus-label"><strong data-tooltip={contentTitleHint(entry.token, reason)}>{contentTitle((titleFailure ?? (pending && art?.name == null ? undefined : art ? art.name : failure)), entry.token, 'Focus name unavailable', pending)}</strong>
        <span className="focus-date" data-tooltip={date == null ? 'Completion date unknown: the list was first read here, or changed other than by a completion.'
          : 'Completed by this date. Focuses are read once a game day, so it may have finished a day or so earlier.'}>
          {date == null ? 'Date not recorded' : formatDate(date)}
        </span>
      </span>
      <FocusFailureNotices errors={errors} />
      <svg className="focus-chevron" viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d="m4 6 4 4 4-4" /></svg>
    </summary>
    <div className="focus-description">
      <div className="focus-duration">{art?.baseDurationDays != null ? <>Base duration <strong>{focusDays(art.baseDurationDays)}</strong></> : 'Duration unavailable'}</div>
      {description && <p>{description}</p>}
    </div>
  </details>;
}, sameCompletion);

const FOCUS_GLIDE_MS = 600;

export function CurrentFocus({ order, art, progress, live, reason, failure, failures, titleFailure, pending }: { order: Pick<FocusOrder, 'current' | 'continuous' | 'paused'>; art: Artwork | undefined; progress?: FocusLiveProgress; live: boolean; reason?: string; failure?: Failure; failures?: readonly Failure[]; titleFailure?: Failure; pending?: boolean }) {
  const errors = focusErrors(failure, failures);
  const token = order.current ?? order.continuous;
  const continuous = !order.current && !!order.continuous;
  const duration = order.current ? art?.baseDurationDays : null;
  const shown = focusProgressView(progress?.at() ?? null, duration, !!order.current || continuous);
  const days = useRef<HTMLSpanElement>(null), left = useRef<HTMLSpanElement>(null), value = useRef<HTMLElement>(null), unit = useRef<HTMLElement>(null);
  const bar = useRef<HTMLSpanElement>(null), banked = useRef<HTMLSpanElement>(null);
  useLayoutEffect(() => {
    if (!progress) return;
    let painted: FocusProgressState | null | undefined;
    const paint = () => {
      const state = progress.at();
      if (state === painted) return;
      painted = state;
      const now = focusProgressView(state, duration, !!order.current || continuous);
      setHidden(days.current, now.days === null); setText(days.current, now.days ?? '');
      if (left.current) {
        setHidden(left.current, now.left === null);
        setAttr(left.current, 'data-tooltip', now.left?.estimated ? 'Estimated at current focus speed.' : '');
        setText(value.current, now.left?.value ?? '');
        setText(unit.current, now.left?.unit ?? '');
      }
      setHidden(bar.current, now.fraction === null);
      setAttr(bar.current, 'aria-valuenow', String(Math.round((now.fraction ?? 0) * 100)));
      const [fill, knob] = bar.current ? [...bar.current.children] as HTMLElement[] : [];
      const glide = reducedMotion() ? 0 : FOCUS_GLIDE_MS;
      if (fill) progressFill.to(fill.style, now.fraction ?? 0, glide);
      if (knob) progressKnob.to(knob.style, now.fraction ?? 0, glide);
      setText(banked.current, now.banked ?? '');
    };
    paint();
    return progress.subscribe(paint);
  }, [progress, duration, order.current, order.continuous]);
  if (!token) {
    return <div className="focus-now is-idle" role="status">
      <span className="focus-art is-large"><span className="focus-missing-art"><Icon name="pause" /></span></span>
      <span className="focus-now-text"><strong>No national focus selected</strong>
        <span ref={banked}>{shown.banked}</span></span>
    </div>;
  }
  const status = continuous ? 'Continuous focus' : order.paused ? 'Paused' : 'In progress';
  const name = contentTitle((titleFailure ?? (pending && art?.name == null ? undefined : art ? art.name : failure)), token, 'Focus name unavailable', pending);
  const timed = !!order.current && duration != null;
  return <div data-detail={failureDetails(errors)} className={`focus-now${order.paused ? ' is-paused' : ''}`} aria-live="polite">
    <FocusArt art={art} size="large" />
    <span className="focus-now-text"><strong data-tooltip={contentTitleHint(token, reason)}>{name}</strong>
      <FocusFailureNotices errors={errors} />
      <span className="focus-now-status">{live && !continuous && !order.paused && <span className="live-dot" />}{status}
        {timed && <span ref={days} className="focus-now-days" hidden={shown.days === null}>{shown.days}</span>}</span></span>
    {timed && <span ref={left} className="focus-now-left" hidden={shown.left === null} data-tooltip={shown.left?.estimated ? 'Estimated at current focus speed.' : undefined}>
      <strong ref={value}>{shown.left?.value}</strong><small ref={unit}>{shown.left?.unit}</small>
    </span>}
    {timed && <span ref={bar} className="focus-progress" role="progressbar" aria-label={`${name} progress`} aria-valuemin={0} aria-valuemax={100} hidden={shown.fraction === null}
      aria-valuenow={Math.round((shown.fraction ?? 0) * 100)}><span /><i /></span>}
  </div>;
}

export default function NationalFocusWidget({ widget, runtime, active, headerSlot, onDiagnostics }: WidgetProps) {
  const menu = useContextMenu('National focus actions');
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryKey(track) ?? undefined;
  const live = widget.time.mode === 'follow';
  const moment = live || widget.time.mode === 'pin';
  const gate = useGate(runtime, widget, SPEC, active);
  const hourAt = useRegionHourAt(runtime, gate.region);
  const request = track && tag && gate.open ? focusRequest(track.source.id, track.source.revision, { branchId: track.branch.id, country: countryFromKey(tag) }) : null;
  const { data, error, retry } = useProduct<FocusReply>(runtime.client, request, active, validateFocusReply);
  const trackHour = (clockHour: number): number | null => hourAt(axisAt(clockHour));
  useShownEachFrame(runtime.clock, () => data ? trackHour(runtime.clock.getHour()) : null);
  const headHour = useHeadHour(runtime.clock, active && live && !!data, clockHour => data ? focusShownAt(data, trackHour(clockHour)) : '');
  const settled = live ? null : trackHour(runtime.clock.getHour());
  const key = data ? focusShownAt(data, live ? trackHour(headHour) : settled) : '';
  const segmentIndex = data && key.startsWith('s') ? Number(key.slice(1)) : -1;
  const segment = data && segmentIndex >= 0 ? data.history.segments[segmentIndex] : null;
  const missing = data && key.startsWith('m') ? data.missing[Number(key.slice(1))] : null;
  const order = segment?.order ?? null;
  const readingHour = segment?.firstHour;
  const readHours = useMemo(() => data ? focusReadHours(data.history) : [], [data]);
  const hours = segmentIndex >= 0 ? readHours[segmentIndex] : undefined;
  const completedHour = (entry: FocusEntry) => hours?.completed[entry.ordinal] ?? entry.completedByHour ?? readingHour!;
  const contentRevision = useContentRevision(runtime.client);
  const detailsIdentity = track ? JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag, contentRevision]) : '';
  const [opened, setOpened] = useState<{ identity: string; client: typeof runtime.client; rows: Set<string> } | null>(null);
  const metadata = useMemo(() => {
    const text = new Map(data?.staticText?.map(item => [item.token, item]));
    return new Map(data?.artwork.map(({ token, baseDurationDays }) => [token,
      { token, baseDurationDays, name: text.get(token)?.name ?? null, description: text.get(token)?.description ?? null, icon: null } satisfies Artwork]));
  }, [data]);
  const continuousMetadata = useMemo(() => new Map(data?.continuousStaticText.map(item => [item.token, item])), [data]);
  const progress = useMemo<FocusLiveProgress | undefined>(() => {
    if (!data || !moment) return undefined;
    let held: { hour: number | null; state: FocusProgressState | null } | null = null;
    const at = () => {
      const hour = live ? trackHour(runtime.clock.getHour()) : settled;
      if (held?.hour !== hour) held = { hour, state: hour === null ? null : focusProgressAt(data.history, hour) };
      return held.state;
    };
    return { at, subscribe: changed => {
      if (!live) return () => {};
      const stops = [runtime.clock.subscribeFrame(changed), runtime.clock.subscribe(changed)];
      return () => { for (const stop of stops) stop(); };
    } };
  }, [data, moment, live, settled, runtime.clock, hourAt]);
  const asked = useRef<{ wanted: NameAt[]; continuous: NameAt[] }>({ wanted: [], continuous: [] });
  const aheadUntil = live && segment ? focusAheadUntil(segment.lastHour + 1, runtime.clock.getSnapshot().hoursPerSecond, AHEAD_SECONDS) : -Infinity;
  const ahead = (asks: NameAt[], asksOf: (order: FocusOrder, hours: SegmentHours) => NameAt[]) => data
    ? focusAsksAhead(asks, data.history.segments, readHours, segmentIndex, aheadUntil, asksOf) : asks;
  const wanted = sameAsks(asked.current.wanted, ahead(focusArtAsks(order, hours, live, moment), (next, held) => focusArtAsks(next, held, live, moment)));
  const continuousWanted = sameAsks(asked.current.continuous, ahead(continuousArtAsks(order, hours), continuousArtAsks));
  asked.current = { wanted, continuous: continuousWanted };
  const openRows = opened?.identity === detailsIdentity && opened.client === runtime.client ? opened.rows : null;
  const detailsWanted = useMemo(() => {
    const rows = openRows ? order?.completed.flatMap(entry => {
      const hour = completedHour(entry);
      return Number.isInteger(hour) && openRows.has(disclosureKey(entry, hour)) ? [{ tag: entry.token, hour }] : [];
    }) ?? [] : [];
    return live ? rows.reverse() : rows;
  }, [openRows, order, hours, readingHour, live]);
  const inRange = key !== 'out';
  const dated = useFocusArtwork(runtime.client, track, tag, wanted, active && inRange, detailsWanted, data?.staticText);
  const continuousDated = useContinuousFocusArtwork(runtime.client, track, tag, continuousWanted, active && inRange, data?.continuousStaticText);
  const shownTexts = useRef<{ identity: string; client: typeof runtime.client; texts: Map<string, string> } | null>(null);
  if (shownTexts.current?.identity !== detailsIdentity || shownTexts.current.client !== runtime.client) {
    shownTexts.current = { identity: detailsIdentity, client: runtime.client, texts: new Map() };
  }
  const texts = shownTexts.current.texts;
  const toggleDetails = (disclosure: string, open: boolean) => setOpened(previous => {
    const rows = previous?.identity === detailsIdentity && previous.client === runtime.client ? previous.rows : new Set<string>();
    if (rows.has(disclosure) === open && previous?.identity === detailsIdentity && previous.client === runtime.client) return previous;
    const next = new Set(rows);
    if (open) next.add(disclosure); else next.delete(disclosure);
    return { identity: detailsIdentity, client: runtime.client, rows: next };
  });
  const at = (database: FocusDatabase, token: string, readHour: number) => {
    const item = (database === 'national' ? dated : continuousDated).read.get(nameKey(readHour, token));
    const fallback = (database === 'national' ? metadata : continuousMetadata).get(token);
    return item && !isFailedFact(item) ? { art: { ...item.artwork, description: item.artwork.description ?? fallback?.description ?? null },
      titleFailure: item.titleFailure, failures: [item.titleFailure, item.imageFailure, item.descriptionFailure].filter(isFailedFact), failure: item.titleFailure ?? item.imageFailure ?? item.descriptionFailure, reason: [item.titlePending ? 'Loading focus name…' : item.titleReason, item.reason].filter(Boolean).join('; ') || undefined, pending: item.titlePending ?? false }
      : { art: fallback, titleFailure: isFailedFact(item) ? item : undefined, failures: isFailedFact(item) ? [item] : [], failure: isFailedFact(item) ? item : undefined, pending: active && !!track && !!tag && item === undefined, reason: active && !!track && !!tag && item === undefined ? 'Loading focus name…'
        : isFailedFact(item) ? failureText(item, 'Focus name request failed.') : 'Focus name was not recorded at this hour.' };
  };
  const unavailable = useMemo(() => {
    const reasons = new Map<string, number>();
    for (const item of [...dated.read.values(), ...continuousDated.read.values()]) {
      const reason = isFailedFact(item) ? failureText(item, 'Game artwork request failed.')
        : item === null ? 'Game artwork was not recorded at this hour.' : [item.titleReason, item.reason, item.imageReason].filter(Boolean).join('; ');
      if (reason) reasons.set(reason, (reasons.get(reason) ?? 0) + 1);
    }
    for (const item of dated.details?.values() ?? []) {
      const reason = isFailedFact(item) ? failureText(item, 'Focus description request failed.')
        : item === null ? 'Focus description was not recorded at this hour.' : item.reason;
      if (reason) reasons.set(reason, (reasons.get(reason) ?? 0) + 1);
    }
    return [...reasons].map(([reason, count]) => ({ reason, count }));
  }, [dated.read, dated.details, continuousDated.read]);
  const diagnostics = useMemo(() => [...(data?.warnings ?? NO_WARNINGS), ...new Set([...dated.warnings, ...continuousDated.warnings]),
    ...unavailable.map(item => `${item.reason} (${item.count})`)], [data, dated.warnings, continuousDated.warnings, unavailable]);
  useDiagnostics(onDiagnostics, diagnostics);
  const list = useRef<HTMLDivElement>(null);
  const listIdentity = useMemo(() => ({}), [detailsIdentity, runtime.client]);
  const count = order?.completed.length ?? 0;
  const listItems = useMemo<FocusListItem[]>(() => {
    const items: FocusListItem[] = order?.completed.map(entry => ({ kind: 'completed', entry })) ?? [];
    if (!moment && order?.current) items.push({ kind: 'unfinished', token: order.current });
    return items;
  }, [order, moment]);
  const seen = useRef(count);
  useLayoutEffect(() => {
    if (live && count > seen.current) list.current?.scrollTo({ top: list.current.scrollHeight, behavior: document.documentElement.dataset.motion === 'reduced' ? 'auto' : 'smooth' });
    seen.current = count;
  }, [live, count]);
  if (!track || !tag) return <WidgetState kind="nothing-chosen" prompt="Load a campaign and choose a country to see its national focuses." />;
  if (!gate.open) return <GateState gate={gate} loading="Loading national focuses…" />;
  if (error) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data) return <WidgetState kind="loading" label="Loading national focuses…" />;
  if (!inRange) return <WidgetState kind="outside" />;
  if (!order || readingHour === undefined) {
    return <WidgetState kind="not-recorded" reason={missing?.reason} />;
  }
  if (!moment && !listItems.length) {
    return <WidgetState kind="empty" icon="target" message="No completed focuses recorded at this point." />;
  }
  const newest = live ? order.completed.at(-1)?.ordinal : undefined;
  const shownFocus = activeFocus(order);
  const currentArt = at(shownFocus?.database ?? 'national', shownFocus?.token ?? '', hours?.active ?? readingHour);
  return <section key={detailsIdentity} className="national-focus" aria-label="National focus">
    {headerSlot && createPortal(<span className="pill focus-header-count">{order.completed.length} completed</span>, headerSlot)}
    {moment && <CurrentFocus order={order} art={currentArt.art} progress={progress} live={live} reason={currentArt.reason} failure={currentArt.failure} failures={currentArt.failures} titleFailure={currentArt.titleFailure} pending={currentArt.pending} />}
    <div className="focus-scroll" ref={list}>
      {listItems.length ? <VirtualList identity={listIdentity} retainExpansion={false} items={listItems} itemKey={item => item.kind === 'completed'
        ? `${item.entry.ordinal}:${completedHour(item.entry)}` : `unfinished:${item.token}`} estimate={52} label="Completed focuses in order" className="focus-list" scrollRef={list}
        isInteractive={isCompletedFocus} rowProps={item => item.kind === 'unfinished' ? { className: 'focus-entry is-unfinished' }
          : ({ className: `focus-entry${item.entry.ordinal === newest ? ' is-newest' : ''}`, ...rowJumpProps(runtime, track.id, item.entry.completedByHour, menu.open) })}>{item => {
        if (item.kind === 'unfinished') return <div className="focus-row">
          <span className="focus-number" aria-hidden="true">{String(order.completed.length + 1).padStart(2, '0')}</span>
          <FocusArt art={currentArt.art} size="small" />
          <span className="focus-label"><strong data-detail={currentArt.failure?.message} data-tooltip={contentTitleHint(item.token, currentArt.reason)}>{contentTitle((currentArt.titleFailure ?? (currentArt.pending && currentArt.art?.name == null ? undefined : currentArt.art?.name)), item.token, 'Focus name unavailable', currentArt.pending)}</strong>
            <span>{order.paused ? 'Paused' : 'In progress'}</span></span>
          <FocusFailureNotices errors={focusErrors(currentArt.failure, currentArt.failures)} />
        </div>;
        const entry = item.entry;
        const readHour = completedHour(entry), textKey = nameKey(readHour, entry.token), disclosure = disclosureKey(entry, readHour);
        const { art, reason, failure, failures, titleFailure, pending } = at('national', entry.token, readHour);
        if (art?.description) texts.set(textKey, art.description);
        const description = art?.description ?? (dated.details?.get(textKey) === undefined ? texts.get(textKey) : undefined);
        return <FocusCompletion entry={entry} art={art} reason={reason} failure={failure} failures={failures} titleFailure={titleFailure} pending={pending} description={description}
          open={!!openRows?.has(disclosure)} onToggle={open => toggleDetails(disclosure, open)} />;
      }}</VirtualList> : <p className="focus-empty">No completed focuses recorded at this point.</p>}
    </div>
    {menu.menu}
  </section>;
}
