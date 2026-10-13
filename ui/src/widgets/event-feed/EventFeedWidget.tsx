import { useCallback, useEffect, useMemo, useState } from 'react';
import type { CSSProperties } from 'react';
import { ChipSet } from '../../components/ChipSet';
import WidgetState from '../../components/WidgetState';
import LogList from '../../charts/log/LogList';
import { logCountryRequests } from '../../charts/log/logData';
import type { LogRow } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { jumpItem } from '../../charts/jump';
import { useChartTokens } from '../../charts/useChartTokens';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch } from '../../dashboard/groups';
import { ideaTargetTags } from '../../dashboard/ideaTooltip';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useStateNamesAt } from '../../dashboard/useStateNamesAt';
import { CATEGORIES, EVENT_ICONS, OPTIONS } from './definition';
import { Icon } from '../../components/Icons';
import { eventCountryName, eventDescription, eventSentence, eventTags, eventTooltip, notRecorded } from './eventFeedData';
import type { FeedEvent, FeedFilter } from './eventFeedData';
import { useEventFeed } from './useEventFeed';
import './eventFeed.css';

const NO_WARNINGS: string[] = [];
const NO_ROWS: readonly LogRow<FeedEvent>[] = [];
const COLUMNS = 'calc(var(--space-6) + var(--space-2) + var(--space-1)) minmax(0, 1fr) auto';

export default function EventFeedWidget({ widget, runtime, active, onDiagnostics, setOption }: WidgetProps) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const tokens = useChartTokens();
  const filtered = widget.countries.kind !== 'all';
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const inBranch = !!track && !!window && window.lastHour >= track.branch.firstHour
    && (widget.time.mode === 'range' ? window.firstHour <= track.branch.lastHour : window.lastHour <= track.branch.lastHour);
  const hour = track && window && inBranch ? Math.min(window.lastHour, track.branch.lastHour) : null;
  const groups = useCountryGroups(runtime, track, active && filtered && inBranch);
  const from = widget.time.mode === 'range' ? window?.firstHour ?? null : null;
  const tagsKey = JSON.stringify(selectedTags(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(tagsKey), [tagsKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const categoriesKey = JSON.stringify(CATEGORIES.filter(category => options[category.value] === true).map(category => category.value));
  const categories = useMemo<ReadonlySet<string>>(() => new Set(JSON.parse(categoriesKey)), [categoriesKey]);
  const artwork = options.artwork !== false;
  const filter = useMemo<FeedFilter>(() => ({ categories, country: match.matches, firstHour: from, allCountries: !filtered }),
    [categories, match, from, filtered]);
  const filterKey = JSON.stringify([categoriesKey, widget.countries, fallback, from]);
  const feed = useEventFeed(runtime, track, active && match.ready && inBranch, hour, filter, filterKey, widget.time.mode === 'follow');
  const [visible, setVisible] = useState<readonly LogRow<FeedEvent>[]>(NO_ROWS);
  const wanted = useMemo(() => logCountryRequests(visible, row => ({ hour: row.item.event.nameHour,
    tags: [...eventTags(row.item.event), ...ideaTargetTags(row.item.item?.idea)] })), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active && inBranch);
  const wantedStates = useMemo(() => visible.flatMap(row => {
    const event = row.item.event;
    return [event.cause?.state, event.strike?.state].flatMap(state => state == null ? [] : [{ hour: event.hour, state }]);
  }), [visible]);
  const stateAt = useStateNamesAt(runtime.client, track, wantedStates, active && inBranch);
  const stateName = useCallback((hour: number, state: number) => stateAt(hour, state).name, [stateAt]);
  const labels = useCallback((row: LogRow<FeedEvent>) => {
    const at = labelAt(row.item.event.nameHour);
    return (tag: string) => {
      const label = at(tag);
      return { ...label, name: eventCountryName(tag, label) };
    };
  }, [labelAt]);
  const chips = CATEGORIES.map(category => ({ value: category.value, label: category.label,
    colour: tokens.categories[category.palette] }));
  // What this recording could not capture is reported with the warnings, never drawn in the feed.
  const unavailable = useMemo(() => feed.data ? notRecorded(feed.data) : NO_WARNINGS, [feed.data]);
  useEffect(() => {
    const notes = [...feed.warnings, ...unavailable];
    onDiagnostics(notes.length ? notes : NO_WARNINGS);
    return () => onDiagnostics(NO_WARNINGS);
  }, [feed.warnings, unavailable, onDiagnostics]);

  if (!track) return <WidgetState kind="empty" icon="clock" message="Load a campaign to see its key moments." />;
  if (!inBranch) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (filtered && groups.error) return <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} />;
  if (feed.error) return <WidgetState kind="error" detail={feed.error} onRetry={feed.retry} />;
  if (match.reason) return <WidgetState kind="unavailable" message={match.reason} />;
  if (!feed.data || !match.ready) return <WidgetState kind="loading" label="Loading the event feed…" />;

  return <div className="log-widget event-feed-widget">
    {log.chips && <div className="log-chips"><ChipSet label="Event categories" chips={chips}
      selected={[...categories]} max={CATEGORIES.length} onToggle={key => setOption?.(key, !categories.has(key))} /></div>}
    <LogList key={feed.identity} pages={feed.pages} label="Event feed" density={log.density} months={log.months}
      animate={log.animate && widget.time.mode === 'follow'} playhead={widget.time.mode === 'follow' ? hour : null}
      columns={artwork ? COLUMNS : 'minmax(0, 1fr) auto'} onVisible={setVisible} onWantedPages={feed.onWantedPages}
      renderRow={row => {
        const at = row.item;
        const label = labels(row);
        const category = CATEGORIES.find(category => category.value === at.event.category)!;
        return <>
          {artwork && <span className="log-art" aria-hidden="true">{at.item?.icon ? <img src={at.item.icon} alt="" />
            : <span className="event-feed-icon" style={{ '--event-colour': tokens.categories[category.palette] } as CSSProperties}><Icon name={EVENT_ICONS[at.event.kind]} /></span>}</span>}
          <span className="log-text">{eventSentence(at, stateName).map((part, index) => 'tag' in part ? <span key={index}>
            <CountryFlagUrl className="log-flag" url={label(part.tag).flag} reason={label(part.tag).flagReason} pending={label(part.tag).flagPending} /><b>{label(part.tag).name}</b>
          </span> : <span key={index}>{part.text}</span>)}</span>
          <span className="log-tag"><i style={{ background: tokens.categories[category.palette] }} />{category.label}</span>
        </>;
      }}
      describe={row => eventDescription(row.item, tag => labels(row)(tag).name, stateName)}
      tooltip={row => eventTooltip(row.item, tag => labels(row)(tag).name, stateName)}
      menu={row => [jumpItem(runtime, track.id, row.hour)]} empty="No recorded events match these settings." />
    {feed.errors.size > 0 && <div className="event-feed-load">Some events couldn’t load. <button type="button" onClick={feed.retryPages}>Retry</button></div>}
  </div>;
}
