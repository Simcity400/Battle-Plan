import { countryNameDetails } from '../../charts/tooltip.ts';
import { useCallback, useMemo } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { ChipSet } from '../../components/ChipSet';
import ArtFallback from '../../components/ArtFallback';
import { CountryFlagUrl } from '../../components/CountryFlag';
import WidgetState from '../../components/WidgetState';
import LogList from '../../charts/log/LogList';
import LogGaps from '../../charts/log/LogGaps';
import { NO_COUNTRY_REQUESTS, sameCountryRequests } from '../../charts/log/logData';
import type { CountryRequest, LogRow } from '../../charts/log/logData';
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
import ScriptedText from '../event-feed/ScriptedText';
import { reasonsOf } from '../event-feed/scriptedGaps.ts';
import type { CardList } from '../event-feed/useCards';
import { OPTIONS } from './definition';
import { activeCount, CONTINUING, costText, decisionAt, decisionDescription, decisionDiagnostics, decisionJumps, decisionName, decisionRow, decisionTooltip,
  lifecycleLine, OPENERS, STATUS_CHOICES, statusLabel, statusTone } from './decisionsData.ts';
import type { DecisionAt, DecisionCard } from './decisionsData';
import '../event-feed/eventFeed.css';
import './decisions.css';
import { useShownEachFrame } from '../../dashboard/widgetPerf';
import { trackHourOf } from '../../dashboard/settledHour';

export default function DecisionsWidget({ widget, runtime, active, onDiagnostics, setOption, headerSlot }: WidgetProps) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
useShownEachFrame(runtime.clock, () => track ? trackHourOf(widget, track, runtime.tracks, runtime.clock.getHour()) : null);
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const filtered = widget.countries.kind !== 'all';
  const oneCountry = widget.countries.kind === 'selected' || (widget.countries.kind === 'tags' && widget.countries.tags.length === 1);
  const groups = useCountryGroups(runtime, track, active && filtered);
  const tagsKey = JSON.stringify(selectedCountryKeys(widget.countries, track));
  const fallback = useMemo<string[]>(() => JSON.parse(tagsKey), [tagsKey]);
  const match = useMemo(() => countryMatch(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0),
    [widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const statusesKey = JSON.stringify(STATUS_CHOICES.filter(status => options[status.value] !== false).map(status => status.value));
  const statuses = useMemo<ReadonlySet<string>>(() => new Set(JSON.parse(statusesKey)), [statusesKey]);
  const artwork = options.artwork !== false;
  const country = filtered ? match.matches : null;
  const scope = useMemo(() => filtered && match.ready ? countryScope(widget.countries, fallback, groups.data, track?.branch.lastHour ?? 0) ?? undefined : undefined,
    [filtered, match.ready, widget.countries, fallback, groups.data, track?.branch.lastHour]);
  const list = useMemo<CardList<DecisionCard, DecisionAt>>(() => ({ product: 'decisions', scope, openers: OPENERS, continuing: CONTINUING, row: decisionRow,
    edges: card => card.occurrence.edges, days: card => card.occurrence.edges.some(edge => edge.days !== null),
    at: (row, hour, breaks) => decisionAt(row, hour, breaks, statuses, country) }), [statuses, country, scope]);
  const cards = useCards(runtime, widget, track, active && match.ready, list);
  const [wanted, setWanted] = useDistinctState<readonly CountryRequest[]>(NO_COUNTRY_REQUESTS, sameCountryRequests);
  const onVisible = useCallback((rows: readonly LogRow<DecisionAt>[]) => setWanted(occurrenceCountryRequests(rows.map(row => row.item.card.occurrence))), [setWanted]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active);
  const nameHour = (row: LogRow<DecisionAt>) => row.item.card.occurrence.edges[0].row.nameHour;
  const label = useCallback((hour: number, tag: string) => {
    const found = labelAt(hour)(tag);
    return { ...found, name: eventCountryName(tag, found) };
  }, [labelAt]);
  const name = (row: LogRow<DecisionAt>) => (tag: string) => label(nameHour(row), tag).name;
  const warnings = useMemo(() => [...cards.notes, ...decisionDiagnostics(cards.pages.flatMap(page => (page.rows ?? []).map(row => row.item.card)))],
    [cards.notes, cards.pages]);
  useDiagnostics(onDiagnostics, warnings);

  if (!track) return <WidgetState kind="empty" icon="ballot" message="Load a campaign to see decisions." />;
  const groupFailure = filtered && groups.error ? <WidgetState kind="error" failure={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (cards.error) return <>{groupFailure}<WidgetState kind="error" failure={cards.error} onRetry={cards.retry} /></>;
  if (match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
  if (cards.loading || !match.ready) return <>{groupFailure}<WidgetState kind="loading" label="Loading decisions…" /></>;

  const renderRow = (row: LogRow<DecisionAt>) => {
    const decision = row.item;
    const { card } = decision;
    const tone = statusTone(decision);
    const cost = costText(decision);
    const found = card.country === null ? null : label(nameHour(row), card.country);
    const flag = found && <CountryFlagUrl className="log-flag" url={found.flag} detail={found.flagDetail} reason={found.flagReason} pending={found.flagPending} />;
    const named = (placeholder: string): ReactNode => !found ? <span className="construction-ellipsis">A country not identified</span>
      : found.known === undefined ? <span className={`skeleton ${placeholder}`} aria-hidden="true" /> : <span className="construction-ellipsis">{found.name}</span>;
    return <>
      {artwork && <span className="log-art" aria-hidden={card.item?.icon ? true : undefined}>{card.item?.icon ? <img src={card.item.icon} alt="" /> : <ArtFallback icon="ballot" name={decisionName(card)} />}</span>}
      <span className="construction-title">
        <span className="construction-building"><span className={`construction-name${card.item?.name ? '' : ' is-muted'}`} data-tooltip={card.item?.name ? undefined : reasonsOf(card.item).nameMissing}>
          <ScriptedText text={decisionName(card)} reason={reasonsOf(card.item).gaps} /></span></span>
        <span className="construction-place">
          {!oneCountry && <span className="construction-place-country">{flag}{named('construction-name-pending is-short')}<span aria-hidden="true">·</span></span>}
          <span className="construction-ellipsis">{lifecycleLine(decision)}</span>
        </span>
      </span>
      {!oneCountry && <span className="construction-country">{flag}{named('construction-name-pending')}</span>}
      <span className="construction-status">
        <span className={`pill${tone === 'neutral' ? '' : ` ${tone}`}`}>{cards.follow && decision.status === 'active' && <i className="live-dot" aria-hidden="true" />}<span>{statusLabel(decision)}</span></span>
      </span>
      {cost === null ? <span className="log-value is-muted" data-tooltip="Not recorded">—</span>
        : <span className={`log-value${cost === 'None' ? ' is-muted' : ''}`}>{cost}</span>}
    </>;
  };
  const held = cards.pages.length > 0 && cards.pages.every(page => page.rows);
  const total = held && statuses.has('active') ? cards.pages.reduce((sum, page) => sum + activeCount(page.rows ?? []), 0) : null;
  const header = headerSlot && total !== null && createPortal(<span className="pill" data-tooltip={cards.feed.missing.length ? 'Count from recorded hours' : undefined}>{total.toLocaleString()} active</span>, headerSlot);
  return <div className={`log-widget decisions-log${oneCountry ? ' is-one-country' : ''}${artwork ? '' : ' is-no-art'}`}>{groupFailure}
    {header}
    {log.chips && <div className="log-chips"><ChipSet label="Decision status" chips={STATUS_CHOICES} selected={[...statuses]} max={STATUS_CHOICES.length}
      onToggle={key => setOption?.(key, !statuses.has(key))} /></div>}
    <LogGaps missing={cards.feed.missing} until={cards.hour} />
    <LogList pages={cards.pages} label="Decisions" density={log.density} months={log.months}
      animate={log.animate && cards.follow} playhead={cards.follow ? cards.hour : null} onVisible={onVisible} onWantedPages={cards.feed.onWantedPages}
      renderRow={renderRow} describe={row => decisionDescription(row.item, name(row))} tooltip={row => decisionTooltip(row.item, name(row)).map(form => ({ ...form,
        ...countryNameDetails(...[row.item.card.country, row.item.card.target].map(tag => tag === null ? null : label(nameHour(row), tag))) }))}
      menu={row => decisionJumps(row.item).map(hour => jumpItem(runtime, track.id, hour))}
      empty="No recorded decisions match these settings." />
  </div>;
}
