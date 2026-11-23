import { countryNameDetails } from '../../charts/tooltip.ts';
import { useCallback, useMemo } from 'react';
import { createPortal } from 'react-dom';
import { ChipSet } from '../../components/ChipSet';
import ArtFallback from '../../components/ArtFallback';
import { CountryFlagUrl } from '../../components/CountryFlag';
import { Icon } from '../../components/Icons';
import WidgetState from '../../components/WidgetState';
import LiveText, { liveTexts } from '../../charts/LiveText';
import LogList from '../../charts/log/LogList';
import { countsAhead } from '../../charts/log/logSource';
import LogGaps from '../../charts/log/LogGaps';
import type { LogRow } from '../../charts/log/logData';
import { logSettings } from '../../charts/log/logOptions';
import { jumpItem } from '../../charts/jump';
import { useCountryLabelsAt } from '../../dashboard/countryFlagsAt';
import { selectedCountryKeys } from '../../dashboard/countrySelection';
import type { WidgetProps } from '../../dashboard/definitions';
import { countryMatch, countryScope } from '../../dashboard/groups';
import { resolveOptions } from '../../dashboard/options';
import { useCountryGroups } from '../../dashboard/useCountryGroups';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useDistinctState } from '../../dashboard/useDistinctState';
import { eventCountryName } from '../event-feed/eventFeedData';
import { occurrenceCountryRequests } from '../event-feed/scriptOccurrences';
import { useCards } from '../event-feed/useCards';
import type { CardList } from '../event-feed/useCards';
import { AWAITING, EVENT_KINDS, OPTIONS } from './definition';
import { awaitingText, choiceAt, choiceDates, choiceDescription, choiceDiagnostics, choiceJumps, choiceRow, choiceTitle, choiceTooltip, isOpen,
  noChoiceText, optionLines } from './eventChoicesData.ts';
import type { ChoiceAt, ChoiceCard } from './eventChoicesData';
import ScriptedText from '../event-feed/ScriptedText';
import { reasonsOf } from '../event-feed/scriptedGaps.ts';
import '../event-feed/eventFeed.css';
import './eventChoices.css';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { trackHourOf } from '../../dashboard/settledHour';
import type { EventFeedKind } from '../../generated/dashboard';

const NO_ROWS: readonly LogRow<ChoiceAt>[] = [];
const sameKeys = (held: readonly LogRow<ChoiceAt>[], next: readonly LogRow<ChoiceAt>[]) => held.length === next.length
  && held.every((row, index) => row.key === next[index].key);
const CONTINUING: readonly EventFeedKind[] = ['eventOptionSelected'];
const cardHeight = (row: LogRow<ChoiceAt>) => 78 + 18 * Math.max(1, row.item.card.item?.options?.length ?? 1);

export default function EventChoicesWidget({ widget, runtime, active, onDiagnostics, setOption, headerSlot }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
useShownEachFrame(runtime.clock, () => track ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) : null);
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const filtered = widget.countries.kind !== 'all';
  const groups = useCountryGroups(runtime, track, active && filtered);
  const tagsKey = JSON.stringify(selectedCountryKeys(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(tagsKey), [tagsKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const kindsKey = JSON.stringify(EVENT_KINDS.filter(kind => options[kind.value] !== false).map(kind => kind.kind));
  const awaiting = options[AWAITING.value] === true;
  const artwork = options.artwork !== false;
  const country = filtered ? match.matches : null;
  const scope = useMemo(() => filtered && match.ready ? countryScope(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0) ?? undefined : undefined,
    [filtered, match.ready, widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const list = useMemo<CardList<ChoiceCard, ChoiceAt>>(() => ({ product: 'event-choices', scope, openers: JSON.parse(kindsKey), continuing: CONTINUING, row: choiceRow,
    edges: card => card.occurrence.edges, days: () => true, at: (row, hour, breaks) => choiceAt(row, hour, breaks, awaiting, country) }),
  [kindsKey, awaiting, country, scope]);
  const cards = useCards(runtime, widget, track, active && match.ready, list);
  const [visible, setVisible] = useDistinctState<readonly LogRow<ChoiceAt>[]>(NO_ROWS, sameKeys);
  const wanted = useMemo(() => occurrenceCountryRequests(visible.map(row => row.item.card.occurrence)), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const nameHour = (row: LogRow<ChoiceAt>) => row.item.card.first.row.nameHour;
  const label = useCallback((hour: number, tag: string) => {
    const found = labelAt(hour)(tag);
    return { ...found, name: eventCountryName(tag, found) };
  }, [labelAt]);
  const name = useCallback((tag: string, hour: number) => label(hour, tag).name, [label]);
  const counted = !filtered && !awaiting && list.openers.length === EVENT_KINDS.length;
  const offset = track?.axisOffset ?? 0;
  const recorded = cards.feed.missing.length ? ' (recorded hours)' : '';
  const shownCount = cards.feed.shown;
  const liveCount = useMemo(() => countsAhead(new Map([[0, shownCount]]), cards.follow ? cards.feed.ahead : [], () => 0), [shownCount, cards.follow, cards.feed.ahead]);
  const totalText = useMemo(() => liveTexts(runtime.clock, () => cards.follow ? Math.floor(runtime.clock.getHour()) + offset : 0, hour => {
    const total = liveCount(hour).get(0)!;
    return new Map([['total', `${total.toLocaleString()} ${total === 1 ? 'event' : 'events'}${recorded}`]]);
  }), [runtime.clock, cards.follow, offset, liveCount, recorded]);
  const warnings = useMemo(() => [...cards.notes, ...choiceDiagnostics(cards.pages.flatMap(page => (page.rows ?? []).map(row => row.item.card)))],
    [cards.notes, cards.pages]);
  useDiagnostics(onDiagnostics, warnings);

  if (!track) return <WidgetState kind="empty" icon="ballot" message="Load a campaign to see event choices." />;
  const groupFailure = filtered && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (cards.error) return <>{groupFailure}<WidgetState kind="error" failure={cards.error} onRetry={cards.retry} /></>;
  if (match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
  if (list.openers.length > 0 && (cards.loading || !match.ready)) return <>{groupFailure}<WidgetState kind="loading" label="Loading event choices…" /></>;

  const chips = [...EVENT_KINDS.map(kind => ({ value: kind.value, label: kind.label })), { value: AWAITING.value, label: AWAITING.label, hint: AWAITING.hint }];
  const selected = chips.filter(chip => chip.value === AWAITING.value ? awaiting : options[chip.value] !== false).map(chip => chip.value);
  const countryLine = (hour: number, tag: string | null) => {
    if (tag === null) return <span className="event-choice-muted">A country not identified</span>;
    const found = label(hour, tag);
    return <span><CountryFlagUrl className="log-flag" url={found.flag} detail={found.flagDetail} reason={found.flagReason} pending={found.flagPending} /><b>{found.name}</b></span>;
  };
  const renderRow = (row: LogRow<ChoiceAt>) => {
    const choice = row.item;
    const { card } = choice;
    const lines = optionLines(choice);
    return <article className="event-choice">
      <div className="event-choice-heading">
        {artwork && <span className="log-art" aria-hidden="true">{card.item?.icon ? <img src={card.item.icon} alt="" /> : <ArtFallback icon="ballot" />}</span>}
        <b className={card.item?.name ? undefined : 'event-choice-muted'} data-tooltip={card.item?.name ? undefined : reasonsOf(card.item).nameMissing}>
          <ScriptedText text={choiceTitle(card)} reason={reasonsOf(card.item).gaps} /></b>
        <small>{choiceDates(choice)}</small>
      </div>
      <div className="event-choice-countries">{countryLine(nameHour(row), card.country)}
        {card.sender && <><span>from</span>{countryLine(nameHour(row), card.sender)}</>}
        {isOpen(choice) && <span className="pill accent">{cards.follow && <i className="live-dot" aria-hidden="true" />}<span>{awaitingText(choice)}</span></span>}</div>
      <ol className="event-choice-options">
        {lines.map(line => <li key={line.key} className={`event-choice-option${line.chosen ? ' is-chosen' : ''}${line.missing ? ' is-missing' : ''}`}>
          {line.chosen ? <Icon name="check" /> : <i aria-hidden="true" />}<span>{line.letter && `${line.letter}. `}<ScriptedText text={line.text} reason={reasonsOf(card.item).gaps} /></span></li>)}
        {!choice.choice && <li className="event-choice-option is-missing"><i aria-hidden="true" /><span>{noChoiceText(choice)}</span></li>}
      </ol>
    </article>;
  };
  const header = headerSlot && counted && createPortal(<span className="pill"><LiveText live={totalText} at="total" /></span>, headerSlot);
  return <div className="log-widget event-choices">{groupFailure}
    {header}
    {log.chips && <div className="log-chips"><ChipSet label="Event choices filters" chips={chips} selected={selected} max={chips.length}
      onToggle={key => setOption?.(key, !selected.includes(key))} /></div>}
    <LogGaps missing={cards.feed.missing} until={cards.hour} />
    <LogList pages={list.openers.length > 0 ? cards.pages : []} label="Event choices" layout="card" density={log.density} months={log.months}
      rowHeight={cardHeight} anchorRows animate={log.animate && cards.follow} playhead={cards.follow ? cards.hour : null} onVisible={setVisible}
      onWantedPages={cards.feed.onWantedPages} renderRow={renderRow} describe={row => choiceDescription(row.item, name)} tooltip={row => {
        const countries: ReturnType<typeof label>[] = [];
        const forms = choiceTooltip(row.item, (tag, hour) => { const found = label(hour, tag); countries.push(found); return found.name; });
        return forms.map(form => ({ ...form, ...countryNameDetails({ nameDetail: form.detail }, ...countries) }));
      }}
      menu={row => choiceJumps(row.item).map(hour => jumpItem(runtime, track.id, hour))}
      empty="No recorded events match these settings." />
  </div>;
}
