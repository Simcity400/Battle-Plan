import { useGate } from '../../dashboard/regionGate';
import { SPEC } from '../../dashboard/regionTime';
import { countryNameDetails } from '../../charts/tooltip.ts';
import { useCallback, useMemo, useRef } from 'react';
import type { CSSProperties } from 'react';
import { ChipSet } from '../../components/ChipSet';
import WidgetState from '../../components/WidgetState';
import LogList from '../../charts/log/LogList';
import { AHEAD_ROWS, aheadEnd, useRevealClock } from '../../charts/reveal';
import LogGaps from '../../charts/log/LogGaps';
import { logCountryRequests } from '../../charts/log/logData';
import type { LogPage, LogRow } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { newestPageRows, stableLogPages } from '../../charts/log/logPages';
import { jumpItem } from '../../charts/jump';
import { useChartTokens } from '../../charts/useChartTokens';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { Icon } from '../../components/Icons';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, countryScope } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { useProduct } from '../../dashboard/products';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useAheadHour } from '../../dashboard/aheadHour';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useLogPages } from '../../dashboard/useLogPages';
import { useStateNamesAt } from '../../dashboard/useStateNamesAt';
import type { LogCut, LogSpec } from '../../dashboard/useLogPages';
import type { EventFeedSpanReply, EventFeedUnavailable } from '../../generated/dashboard';
import { CATEGORIES, EVENT_ICONS, OPTIONS } from './definition';
import { eventCountryName, eventDescription, eventSentence, eventTags, eventTooltip, familyReveals, familyRows, familyShown, FEED_PAGE_SIZE, feedFacets, feedRow, kindsRequest, pageNotes, pagesNoted,
  spanNotes, validateSpan, withFamilyRows } from './eventFeedData';
import type { FeedEvent } from './eventFeedData';
import ScriptedText from './ScriptedText';
import { reasonsOf } from './scriptedGaps.ts';
import './eventFeed.css';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { trackHourOf } from '../../dashboard/settledHour';
import { useDistinctState } from '../../dashboard/useDistinctState';
import { productRequest } from '../../dashboard/request.ts';

const NO_ROWS: readonly LogRow<FeedEvent>[] = [];
const sameKeys = (held: readonly LogRow<FeedEvent>[], next: readonly LogRow<FeedEvent>[]) => held.length === next.length
  && held.every((row, index) => row.key === next[index].key);
const NO_PAGES: readonly LogPage<FeedEvent>[] = [];
const COLUMNS = 'calc(var(--space-6) + var(--space-2) + var(--space-1)) minmax(0, 1fr) auto';

export default function EventFeedWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const gate = useGate(runtime, widget, SPEC, active);
useShownEachFrame(runtime.clock, () => track ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) : null);
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const tokens = useChartTokens();
  const follow = widget.time.mode === 'follow';
  const filtered = widget.countries.kind !== 'all';
  const window = track ? resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour()) : null;
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const offset = track?.axisOffset ?? 0;
  const cut: LogCut = follow ? { follow: true, offset } : { follow: false, hour: widget.time.mode === 'campaign' ? null : window?.lastHour ?? null };
  const categoriesKey = JSON.stringify(CATEGORIES.filter(category => options[category.value] === true).map(category => category.value));
  const categories = useMemo<ReadonlySet<string>>(() => new Set(JSON.parse(categoriesKey)), [categoriesKey]);
  const artwork = options.artwork !== false;
  const groups = useCountryGroups(runtime, track, active && filtered);
  const tagsKey = JSON.stringify(selectedCountryKeys(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(tagsKey), [tagsKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const span = useProduct<EventFeedSpanReply>(runtime.client, track ? productRequest({ sourceId: track.source.id, revision: track.source.revision, product: 'event-feed-span',
    params: { branchId: track.branch.id, categories: [...categories] } }) : null, active, validateSpan, track?.source);
  const coded = useProduct<EventFeedSpanReply>(runtime.client, track ? kindsRequest(track) : null, active, validateSpan, track?.source);
  const kinds = coded.data?.kinds;
  const facets = useMemo(() => kinds ? feedFacets(kinds, categories) : null, [kinds, categories]);
  const spanOnly = !!kinds && !facets;
  const scope = useMemo(() => filtered && match.ready ? countryScope(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0) : null,
    [filtered, match.ready, widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const logSpan = track ? gate.span(from ?? track.branch.firstHour, track.branch.lastHour) : null;
  const spec = useMemo<LogSpec | null>(() => track && logSpan && facets && (!filtered || scope) ? { source: track.source, branchId: track.branch.id, log: 'feed-events',
    product: 'event-feed', facets, ...(scope ? { scope: scope?.flatMap(run => { const held = gate.span(run.firstHour, run.lastHour); return held ? [{ ...run, ...held }] : []; }) } : {}), ...logSpan } : null,
  [track, facets, logSpan?.firstHour, logSpan?.lastHour, filtered, scope]);
  const feed = useLogPages<FeedEvent, EventFeedUnavailable>(runtime.client, runtime.clock, spec, cut, active && match.ready, feedRow, FEED_PAGE_SIZE, pageNotes);
  const country = filtered ? match.matches : null;
  const family = useMemo(() => span.data ? familyRows(span.data, categories, from, country) : NO_ROWS, [span.data, categories, from, country]);
  const reveals = useMemo(() => familyReveals(family), [family]);
  const familyEnd = useCallback((clockHour: number) => {
    const { hoursPerSecond, playing } = runtime.clock.getSnapshot();
    return aheadEnd(reveals, Math.floor(clockHour) + offset, hoursPerSecond, playing) - offset;
  }, [runtime.clock, reveals, offset]);
  const familyBlock = useAheadHour(runtime.clock, active && follow && spanOnly && !!span.data, familyEnd);
  const through = !spanOnly ? feed.through : cut.follow ? familyBlock.until - 1 + offset : cut.hour;
  const shownFamily = familyShown(family, through);
  const trackHour = useCallback((clockHour: number) => Math.floor(clockHour) + offset, [offset]);
  const reveal = useRevealClock(runtime.clock, follow, trackHour);
  const merged = useRef<readonly LogPage<FeedEvent>[]>(NO_PAGES);
  const pages = useMemo(() => merged.current = stableLogPages(merged.current, withFamilyRows(feed.pages, family, shownFamily)),
    [feed.pages, family, shownFamily]);
  const [visible, setVisible] = useDistinctState<readonly LogRow<FeedEvent>[]>(NO_ROWS, sameKeys);
  const newest = useMemo(() => follow ? newestPageRows(pages, feed.ahead.length + 2 * AHEAD_ROWS) : NO_ROWS, [follow, pages, feed.ahead.length]);
  const newestKeys = useMemo(() => new Set(newest.map(row => row.key)), [newest]);
  const onVisible = useCallback((rows: readonly LogRow<FeedEvent>[]) => {
    if (follow && rows.every(row => newestKeys.has(row.key))) return;
    setVisible(rows);
  }, [follow, newestKeys, setVisible]);
  const named = useMemo(() => newest.length ? [...newest, ...visible.filter(row => !newestKeys.has(row.key))] : visible, [newest, newestKeys, visible]);
  const wanted = useMemo(() => logCountryRequests(named, row => ({ hour: row.item.event.nameHour, tags: eventTags(row.item.event) })), [named]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const wantedStates = useMemo(() => named.flatMap(row => {
    const event = row.item.event;
    return [event.cause?.state, event.strike?.state].flatMap(state => state == null ? [] : [{ hour: event.hour, state }]);
  }), [named]);
  const stateAt = useStateNamesAt(runtime.client, track, wantedStates, active);
  const stateName = useCallback((hour: number, state: number) => stateAt(hour, state).name, [stateAt]);
  const labels = useCallback((row: LogRow<FeedEvent>) => {
    const at = labelAt(row.item.event.nameHour);
    return (tag: string) => {
      const label = at(tag);
      return { ...label, name: eventCountryName(tag, label) };
    };
  }, [labelAt]);
  const notes = useMemo(() => span.data ? [...new Set([...spanNotes(span.data), ...pagesNoted(feed.notes)])] : null, [span.data, feed.notes]);
  useDiagnostics(onDiagnostics, notes);

  if (!track) return <WidgetState kind="empty" icon="clock" message="Load a campaign to see its key moments." />;
  const groupFailure = filtered && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  const spanFailure = span.error ?? coded.error;
  if (spanFailure) return <WidgetState kind="error" failure={spanFailure} onRetry={() => { span.retry(); coded.retry(); }} />;
  if (feed.error) return <WidgetState kind="error" failure={feed.error} onRetry={feed.retry} />;
  if (match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
  if (!kinds || !match.ready || (spec && !feed.counts)) return <>{groupFailure}<WidgetState kind="loading" label="Loading the event feed…" /></>;
  const hour = cut.follow ? Math.floor(runtime.clock.getHour()) + offset : cut.hour;
  const empty = categories.size > 0 && !span.data ? <span data-state="loading">Loading the event feed…</span>
    : 'No recorded events match these settings.';
  const chips = CATEGORIES.map(category => ({ value: category.value, label: category.label, colour: tokens.categories[category.palette] }));
  return <div className="log-widget event-feed-widget">{groupFailure}
    {log.chips && <div className="log-chips"><ChipSet label="Event categories" chips={chips}
      selected={[...categories]} max={CATEGORIES.length} onToggle={key => setOption?.(key, !categories.has(key))} /></div>}
    <LogGaps missing={feed.missing} until={hour} />
    <LogList pages={categories.size ? pages : NO_PAGES} label="Event feed" density={log.density} months={log.months}
      animate={log.animate && follow} playhead={follow ? hour : null} reveal={reveal}
      columns={artwork ? COLUMNS : 'minmax(0, 1fr) auto'} onVisible={onVisible} onWantedPages={feed.onWantedPages}
      renderRow={row => {
        const at = row.item;
        const label = labels(row);
        const category = CATEGORIES.find(category => category.value === at.event.category)!;
        return <>
          {artwork && <span className="log-art" aria-hidden="true">{at.item?.icon ? <img src={at.item.icon} alt="" />
            : <span className="event-feed-icon" style={{ '--event-colour': tokens.categories[category.palette] } as CSSProperties}><Icon name={EVENT_ICONS[at.event.kind]} /></span>}</span>}
          <span className="log-text">{eventSentence(at, stateName).map((part, index) => 'tag' in part ? <span key={index}>
            <CountryFlagUrl className="log-flag" url={label(part.tag).flag} detail={label(part.tag).flagDetail} reason={label(part.tag).flagReason} pending={label(part.tag).flagPending} /><b>{label(part.tag).name}</b>
          </span> : <span key={index}><ScriptedText text={part.text} reason={reasonsOf(at.item).gaps} /></span>)}</span>
          <span className="log-tag"><i style={{ background: tokens.categories[category.palette] }} />{category.label}</span>
        </>;
      }}
      describe={row => eventDescription(row.item, tag => labels(row)(tag).name, stateName)}
      tooltip={row => {
        const countries: ReturnType<ReturnType<typeof labels>>[] = [];
        const forms = eventTooltip(row.item, tag => { const country = labels(row)(tag); countries.push(country); return country.name; }, stateName);
        return forms.map(form => ({ ...form, ...countryNameDetails({ nameDetail: form.detail }, ...countries) }));
      }}
      menu={row => [jumpItem(runtime, track.id, row.hour)]} empty={empty} />
  </div>;
}
