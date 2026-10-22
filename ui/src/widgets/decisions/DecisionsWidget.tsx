import { useCallback, useMemo, useState } from 'react';
import type { ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { ChipSet } from '../../components/ChipSet';
import ArtFallback from '../../components/ArtFallback';
import { CountryFlagUrl } from '../../components/CountryFlag';
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
import { eventCountryName } from '../event-feed/eventFeedData';
import { occurrenceCountryRequests, recordingBreaks, scriptNotRecorded } from '../event-feed/scriptOccurrences';
import type { RecordingBreak } from '../event-feed/scriptOccurrences';
import { useScriptOccurrences } from '../event-feed/useScriptOccurrences';
import { OPTIONS } from './definition';
import { activeCount, costText, decisionAt, decisionDescription, decisionDiagnostics, decisionJumps, decisionName, decisionTooltip,
  lifecycleLine, prepareDecisions, STATUS_CHOICES, statusLabel, statusTone } from './decisionsData';
import type { DecisionAt, DecisionFilter } from './decisionsData';
import '../event-feed/eventFeed.css';
import '../construction-log/constructionLog.css';
import './decisions.css';

const NO_ROWS: readonly LogRow<DecisionAt>[] = [];
const NO_KINDS: readonly never[] = [];

/** Every decision and mission the chosen countries took, as Construction log's
 * rows: newest first by the date selected, each with its art, name over how it
 * ran or ended, the country, a status pill and what it is proven to have cost.
 * A row keeps its place while its status changes with playback. */
export default function DecisionsWidget({ widget, runtime, active, onDiagnostics, setOption, headerSlot }: WidgetProps) {
  const live = widget.time.mode === 'follow';
  usePlaybackHour(runtime.clock, active && live);
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const log = logSettings(options);
  const filtered = widget.countries.kind !== 'all';
  // One country is named by the header chip, so its rows leave the country out.
  const oneCountry = widget.countries.kind === 'selected' || (widget.countries.kind === 'tags' && widget.countries.tags.length === 1);
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
  const statusesKey = JSON.stringify(STATUS_CHOICES.filter(status => options[status.value] !== false).map(status => status.value));
  const statuses = useMemo<ReadonlySet<string>>(() => new Set(JSON.parse(statusesKey)), [statusesKey]);
  const artwork = options.artwork !== false;
  const filter = useMemo<DecisionFilter>(() => ({ country: match.matches, allCountries: !filtered, firstHour: from }), [match, filtered, from]);
  const prepare = useCallback((reply: Parameters<typeof prepareDecisions>[0]) => prepareDecisions(reply, filter), [filter]);
  const breaksKey = track ? JSON.stringify(recordingBreaks(track.branch)) : '[]';
  const breaks = useMemo<RecordingBreak[]>(() => JSON.parse(breaksKey), [breaksKey]);
  const at = useCallback((row: LogRow<DecisionAt>, at: number) => decisionAt(row, at, breaks, statuses), [breaks, statuses]);
  const filterKey = JSON.stringify([widget.countries, fallback, from]);
  const feed = useScriptOccurrences(runtime, track, active && match.ready && inBranch, hour, 'decisions', NO_KINDS, from, filterKey,
    prepare, at, activeCount);
  const [visible, setVisible] = useState<readonly LogRow<DecisionAt>[]>(NO_ROWS);
  const wanted = useMemo(() => occurrenceCountryRequests(visible.map(row => row.item.card.occurrence)), [visible]);
  const { labelAt } = useCountryLabelsAt(runtime.client, track, wanted, active && inBranch);
  const nameHour = (row: LogRow<DecisionAt>) => row.item.card.occurrence.edges[0].row.nameHour;
  const label = useCallback((hour: number, tag: string) => {
    const found = labelAt(hour)(tag);
    return { ...found, name: eventCountryName(tag, found) };
  }, [labelAt]);
  const name = (row: LogRow<DecisionAt>) => (tag: string) => label(nameHour(row), tag).name;
  const warnings = useMemo(() => [...feed.warnings, ...(feed.data ? scriptNotRecorded(feed.data) : []), ...decisionDiagnostics(feed.replies)],
    [feed.warnings, feed.data, feed.replies]);
  useDiagnostics(onDiagnostics, warnings);

  const feedError = useHeldFailure(feed.error, feed.data !== undefined, feed.retry);
  if (!track) return <WidgetState kind="empty" icon="ballot" message="Load a campaign to see decisions." />;
  if (!inBranch) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  const groupFailure = filtered && groups.error ? <WidgetState kind="error" detail={groups.error} onRetry={groups.retry} /> : null;
  if (groupFailure && !groups.data) return groupFailure;
  if (feedError) return <>{groupFailure}<WidgetState kind="error" detail={feedError} onRetry={feed.retry} /></>;
  if (match.reason) return <>{groupFailure}<WidgetState kind="unavailable" message={match.reason} /></>;
  if (!feed.data || !match.ready) return <>{groupFailure}<WidgetState kind="loading" label="Loading decisions…" /></>;

  const renderRow = (row: LogRow<DecisionAt>) => {
    const decision = row.item;
    const { card } = decision;
    const tone = statusTone(decision);
    const cost = costText(decision);
    const found = card.country === null ? null : label(nameHour(row), card.country);
    const flag = found && <CountryFlagUrl className="log-flag" url={found.flag} reason={found.flagReason} pending={found.flagPending} />;
    const named = (placeholder: string): ReactNode => !found ? <span className="construction-ellipsis">A country not identified</span>
      : found.known === undefined ? <span className={`skeleton ${placeholder}`} aria-hidden="true" /> : <span className="construction-ellipsis">{found.name}</span>;
    return <>
      {artwork && <span className="log-art" aria-hidden="true">{card.item?.icon ? <img src={card.item.icon} alt="" /> : <ArtFallback icon="ballot" />}</span>}
      <span className="construction-title">
        <span className="construction-building"><span className={`construction-name${card.item?.name ? '' : ' is-muted'}`}>{decisionName(card)}</span></span>
        <span className="construction-place">
          {!oneCountry && <span className="construction-place-country">{flag}{named('construction-name-pending is-short')}<span aria-hidden="true">·</span></span>}
          <span className="construction-ellipsis">{lifecycleLine(decision)}</span>
        </span>
      </span>
      {!oneCountry && <span className="construction-country">{flag}{named('construction-name-pending')}</span>}
      <span className="construction-status">
        <span className={`pill${tone === 'neutral' ? '' : ` ${tone}`}`}>{live && decision.status === 'active' && <i className="live-dot" aria-hidden="true" />}<span>{statusLabel(decision)}</span></span>
      </span>
      {cost === null ? <span className="log-value is-muted" data-tooltip="Not recorded">—</span>
        : <span className={`log-value${cost === 'None' ? ' is-muted' : ''}`}>{cost}</span>}
    </>;
  };
  const header = headerSlot && feed.total !== null && statuses.has('active') && createPortal(<span className="pill">{feed.total.toLocaleString()} active</span>, headerSlot);
  return <div className={`log-widget construction-log decisions-log${oneCountry ? ' is-one-country' : ''}${artwork ? '' : ' is-no-art'}`}>{groupFailure}
    {header}
    {log.chips && <div className="log-chips"><ChipSet label="Decision status" chips={STATUS_CHOICES} selected={[...statuses]} max={STATUS_CHOICES.length}
      onToggle={key => setOption?.(key, !statuses.has(key))} /></div>}
    <LogList key={feed.identity} pages={feed.pages} label="Decisions" density={log.density} months={log.months} preparedCoverage={feed.prefix?.coverage}
      animate={log.animate && live} playhead={live ? hour : null} onVisible={setVisible} onWantedPages={feed.onWantedPages}
      renderRow={renderRow} describe={row => decisionDescription(row.item, name(row))} tooltip={row => decisionTooltip(row.item, name(row))}
      menu={row => decisionJumps(row.item).map(hour => jumpItem(runtime, track.id, hour))} empty={match.known(from ?? track.branch.firstHour, hour!) ? "No recorded decisions match these settings." : MEMBERSHIP_PREPARING} />
    {feed.errors.size > 0 && <div className="event-feed-load">Some decisions couldn’t load. <button type="button" onClick={feed.retryPages}>Retry</button></div>}
  </div>;
}
