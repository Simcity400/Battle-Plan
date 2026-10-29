import { useCallback, useMemo, useState } from 'react';
import { createPortal } from 'react-dom';
import { ChipSet } from '../../components/ChipSet';
import ArtFallback from '../../components/ArtFallback';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { Icon } from '../../components/Icons';
import WidgetState from '../../components/WidgetState';
import { useHeldFailure } from '../../dashboard/heldFailures';
import LogList from '../../charts/log/LogList';
import type { LogRow } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { jumpItem } from '../../charts/jump';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedTags } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, MEMBERSHIP_PREPARING } from '../../dashboard/groups';
import { resolveWindow } from '../../dashboard/model';
import { resolveOptions } from '../../dashboard/options';
import { usePlaybackHour } from '../../dashboard/playback';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { EventFeedKind } from '../../generated/dashboard';
import { eventCountryName } from '../event-feed/eventFeedData';
import { occurrenceCountryRequests, recordingBreaks, scriptNotRecorded } from '../event-feed/scriptOccurrences';
import type { RecordingBreak } from '../event-feed/scriptOccurrences';
import { useScriptOccurrences } from '../event-feed/useScriptOccurrences';
import { AWAITING, EVENT_KINDS, OPTIONS } from './definition';
import { awaitingText, choiceAt, choiceDates, choiceDescription, choiceDiagnostics, choiceJumps, choiceTitle, choiceTooltip, isOpen,
  noChoiceText, optionLines, prepareChoices } from './eventChoicesData';
import type { ChoiceAt, ChoiceFilter } from './eventChoicesData';
import '../event-feed/eventFeed.css';
import '../peace-deals/peaceDeals.css';
import './eventChoices.css';

const NO_ROWS: readonly LogRow<ChoiceAt>[] = [];
const count = (rows: readonly LogRow<ChoiceAt>[]) => rows.length;
const cardHeight = (row: LogRow<ChoiceAt>) => 78 + 18 * Math.max(1, row.item.card.item?.options?.length ?? 1);

/** Each recorded delivery of a scripted or news event to a country as a card:
 * its picture and title, the country and its sender, when it fired and was
 * chosen, and every authored option with the chosen one marked. */
export default function EventChoicesWidget({ widget, runtime, active, onDiagnostics, setOption, headerSlot }: WidgetProps) {
  const live = widget.time.mode === 'follow';
  usePlaybackHour(runtime.clock, active && live);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
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
  const kindsKey = JSON.stringify(EVENT_KINDS.filter(kind => options[kind.value] !== false).map(kind => kind.kind));
  const kinds = useMemo<EventFeedKind[]>(() => JSON.parse(kindsKey), [kindsKey]);
  const awaiting = options[AWAITING.value] === true;
  const artwork = options.artwork !== false;
  const filter = useMemo<ChoiceFilter>(() => ({ kinds: new Set(kinds), country: match.matches, allCountries: !filtered, firstHour: from }),
    [kinds, match, filtered, from]);
  const prepare = useCallback((reply: Parameters<typeof prepareChoices>[0]) => prepareChoices(reply, filter), [filter]);
  const breaksKey = track ? JSON.stringify(recordingBreaks(track.branch)) : '[]';
  const breaks = useMemo<RecordingBreak[]>(() => JSON.parse(breaksKey), [breaksKey]);
  const at = useCallback((row: LogRow<ChoiceAt>, at: number) => choiceAt(row, at, breaks, awaiting), [breaks, awaiting]);
  const filterKey = JSON.stringify([kindsKey, widget.countries, fallback, from]);
  const feed = useScriptOccurrences(runtime, track, active && match.ready && inBranch && kinds.length > 0, hour, 'event-choices',
    kinds, from, filterKey, prepare, at, count);
  const [visible, setVisible] = useState<readonly LogRow<ChoiceAt>[]>(NO_ROWS);
  const wanted = useMemo(() => occurrenceCountryRequests(visible.map(row => row.item.card.occurrence)), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active && inBranch);
  const nameHour = (row: LogRow<ChoiceAt>) => row.item.card.first.row.nameHour;
  const label = useCallback((hour: number, tag: string) => {
    const found = labelAt(hour)(tag);
    return { ...found, name: eventCountryName(tag, found) };
  }, [labelAt]);
  const name = useCallback((tag: string, hour: number) => label(hour, tag).name, [label]);
  const warnings = useMemo(() => [...feed.warnings, ...(feed.data ? scriptNotRecorded(feed.data) : []), ...choiceDiagnostics(feed.replies)],
    [feed.warnings, feed.data, feed.replies]);
  useDiagnostics(onDiagnostics, warnings);

  const feedError = useHeldFailure(feed.error, feed.data !== undefined, feed.retry);
  if (!track) return <WidgetState kind="empty" icon="ballot" message="Load a campaign to see event choices." />;
  if (!inBranch) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  const groupFailure = filtered && groups.error ? <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (feedError) return <>{groupFailure}<WidgetState kind="error" detail={feedError} onRetry={feed.retry} /></>;
  if (match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
  if (kinds.length > 0 && (!feed.data || !match.ready)) return <>{groupFailure}<WidgetState kind="loading" label="Loading event choices…" /></>;

  const chips = [...EVENT_KINDS.map(kind => ({ value: kind.value, label: kind.label })), { value: AWAITING.value, label: AWAITING.label, hint: AWAITING.hint }];
  const selected = chips.filter(chip => chip.value === AWAITING.value ? awaiting : options[chip.value] !== false).map(chip => chip.value);
  const country = (hour: number, tag: string | null) => {
    if (tag === null) return <span className="peace-muted">A country not identified</span>;
    const found = label(hour, tag);
    return <span><CountryFlagUrl className="log-flag" url={found.flag} reason={found.flagReason} pending={found.flagPending} /><b>{found.name}</b></span>;
  };
  const renderRow = (row: LogRow<ChoiceAt>) => {
    const choice = row.item;
    const { card } = choice;
    const lines = optionLines(choice);
    return <article className="peace-deal event-choice">
      <div className="event-choice-heading">
        {artwork && <span className="log-art" aria-hidden="true">{card.item?.icon ? <img src={card.item.icon} alt="" /> : <ArtFallback icon="ballot" />}</span>}
        <b className={card.item?.name ? undefined : 'peace-muted'}>{choiceTitle(card)}</b>
        <small>{choiceDates(choice)}</small>
      </div>
      <div className="event-choice-countries">{country(nameHour(row), card.country)}
        {card.sender && <><span>from</span>{country(nameHour(row), card.sender)}</>}
        {isOpen(choice) && <span className="pill accent">{live && <i className="live-dot" aria-hidden="true" />}<span>{awaitingText(choice)}</span></span>}</div>
      <ol className="event-choice-options">
        {lines.map(line => <li key={line.key} className={`event-choice-option${line.chosen ? ' is-chosen' : ''}${line.missing ? ' is-missing' : ''}`}>
          {line.chosen ? <Icon name="check" /> : <i aria-hidden="true" />}<span>{line.letter && `${line.letter}. `}{line.text}</span></li>)}
        {!choice.choice && <li className="event-choice-option is-missing"><i aria-hidden="true" /><span>{noChoiceText(choice)}</span></li>}
      </ol>
    </article>;
  };
  const header = headerSlot && feed.total !== null && createPortal(<span className="pill">{feed.total.toLocaleString()} {feed.total === 1 ? 'event' : 'events'}</span>, headerSlot);
  return <div className="log-widget peace-deals event-choices">{groupFailure}
    {header}
    {log.chips && <div className="log-chips"><ChipSet label="Event choices filters" chips={chips} selected={selected} max={chips.length}
      onToggle={key => setOption?.(key, !selected.includes(key))} /></div>}
    <LogList key={feed.identity} pages={kinds.length > 0 ? feed.pages : []} label="Event choices" layout="card" density={log.density} months={log.months}
      preparedCoverage={feed.prefix?.coverage} rowHeight={cardHeight} anchorRows
      animate={log.animate && live} playhead={live ? hour : null} onVisible={setVisible} onWantedPages={feed.onWantedPages}
      renderRow={renderRow} describe={row => choiceDescription(row.item, name)} tooltip={row => choiceTooltip(row.item, name)}
      menu={row => choiceJumps(row.item).map(hour => jumpItem(runtime, track.id, hour))} empty={!kinds.length || match.known(from ?? track.branch.firstHour, hour!) ? "No recorded events match these settings." : MEMBERSHIP_PREPARING} />
    {feed.errors.size > 0 && <div className="event-feed-load">Some events couldn’t load. <button type="button" onClick={feed.retryPages}>Retry</button></div>}
  </div>;
}
