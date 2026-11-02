import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import ArtFallback from '../../components/ArtFallback';
import { useContextMenu } from '../../components/ContextMenu';
import { Icon } from '../../components/Icons';
import { Popover, PopoverHeading, usePopover } from '../../components/Popover';
import ListSkeleton from '../../components/Skeleton';
import WidgetState from '../../components/WidgetState';
import { rowJumpProps } from '../../charts/jump';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import { PreparedLogCoverage } from '../../charts/log/LogList';
import { pendingAt, preparedPrefix } from '../../dashboard/preparedPrefix';
import { hideTooltip, showTooltip } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { useParityRows } from '../../dashboard/parityRows';
import { contentTitle } from '../../dashboard/contentTitles';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackHour } from '../../dashboard/playback';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import ResearchExplanation from '../research/ResearchExplanation';
import { ResearchWithDoctrines } from '../research/DoctrinePanel';
import { researchNumber } from '../research/researchBreakdown';
import { researchArt, researchArtwork, researchAt, researchFolderAt, researchFolderLooks, researchRecordedTemplates } from '../research/researchData';
import { researchJourney } from '../research/researchJourney';
import { coverageAt, currentTooltip, researchDate } from '../research/researchLayout';
import type { ResearchText } from '../research/researchLayout';
import { historyTokens, useResearchReply } from '../research/useResearch';
import { useTechnologyArtwork } from '../research/useTechnologyArtwork';
import { activeSlots } from './activeResearchData';
import { useActiveResearch } from './useActiveResearch';
import ResearchGrants from './ResearchGrantInventory';
import type { ActiveSlot } from './activeResearchData';
import '../national-focus/nationalFocus.css';
import '../research/research.css';
import './activeResearch.css';

const NO_WARNINGS: string[] = [];

/** One card per research slot open at the hour shown: what it researches, its
 * latest read progress, the game's estimate and idle slots' saved days. */
export default function ActiveResearchWidget(props: WidgetProps) {
  return <ResearchWithDoctrines {...props} body={ActiveResearchBody} />;
}
function ActiveResearchBody({ widget, runtime, active, onDiagnostics, headerSlot }: WidgetProps) {
  const parity = useRef<HTMLElement>(null);
  const menu = useContextMenu('Active research actions');
  const owner = useId();
  const playbackHour = usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const live = widget.time.mode === 'follow';
  const { track, tag, data: currentData, answeredHour, grants, error, retry, historyPreparing, countdownPreparing } = useActiveResearch(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, playbackHour);
  const hour = window && Math.floor(window.lastHour);
  const history = useResearchReply(widget, runtime, active && !historyPreparing);
  const data = !historyPreparing && history.data && hour != null && !pendingAt(preparedPrefix(history.data), hour) ? history.data : currentData;
  const { categories } = useChartTokens();
  // Reached slots answer their own hour; assignment history answers the asked one.
  const slotHour = data === currentData ? answeredHour ?? hour : hour;
  // The bonus inventory answers the reached read's hour: shown beside cards of that hour only.
  const grantsShown = answeredHour === slotHour ? grants : undefined;
  const slots = useMemo(() => data && slotHour != null ? researchAt(data.history, slotHour) : null, [data, slotHour]);
  const cards = useMemo(() => slots && data && slotHour != null ? activeSlots(slots, coverageAt(data.history.coverage, slotHour), slotHour) : [], [slots, data, slotHour]);
  if (import.meta.env.MODE === 'parity') useParityRows(parity, () => ({ kind: 'windowed', label: 'Active research', complete: true, total: cards.length,
    rows: cards.map((card, index) => ({ index, key: `slot:${card.slot.index}`, item: card.kind === 'idle' ? { ...card, savedRaw: card.savedRaw.toString() } : card })) }));
  const looks = useMemo(() => data ? researchFolderLooks(data, historyTokens(data.history), categories) : new Map(), [data, categories]);
  const tokens = useMemo(() => [...new Set(cards.flatMap(card => card.kind === 'researching' ? [card.token] : []))], [cards]);
  const recorded = useMemo(() => researchRecordedTemplates(data?.history), [data]);
  const images = useTechnologyArtwork(runtime.client, track, tokens, active && !!data, recorded);
  const diagnostics = useMemo(() => [...new Set([...(data?.warnings ?? NO_WARNINGS), ...images.warnings])], [data, images.warnings]);
  useDiagnostics(onDiagnostics, diagnostics);
  const artwork = useMemo(() => researchArtwork(data, images.artwork), [data, images.artwork]);
  const text = useMemo<ResearchText>(() => ({
    name: (token, databaseIndex) => contentTitle(researchArt(artwork, token, databaseIndex)?.name, token, 'Technology name unavailable'),
    icon: token => artwork.get(token)?.icon ?? undefined,
    artworkFailure: token => images.failures?.get(token),
    folder: (token, databaseIndex) => researchFolderAt(looks, token, databaseIndex),
  }), [artwork, looks, images.failures]);
  const untip = useCallback(() => hideTooltip(owner), [owner]);
  useEffect(() => untip, [untip]);
  const breakdown = usePopover('start');
  // An open breakdown names one assignment of the recording, branch and country it was opened for.
  const scope = JSON.stringify([track?.source.id, track?.source.revision, track?.branch.id, tag]);
  const [stored, setShown] = useState<{ scope: string; slotIndex: number; token: string; firstObservedHour: number | null } | null>(null);
  const shown = stored?.scope === scope ? stored : null;
  const { close } = breakdown;
  useEffect(() => close(), [close, scope]);
  const historySlots = useMemo(() => history.data && hour != null ? researchAt(history.data.history, hour) : null, [history.data, hour]);
  const researching = cards.filter(card => card.kind === 'researching').length;
  const header = headerSlot && cards.length > 0 && createPortal(<span className="pill">{researching} of {cards.length} researching</span>, headerSlot);
  if (!track || !tag) return <div className="dash-empty" role="status"><span className="empty-icon"><Icon name="flask" /></span><p>Load a campaign and choose a country to see its research.</p></div>;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <div className="dash-empty" role="status">This time is outside the recording.</div>;
  if (error && !data) return <WidgetState kind="error" failure={error} onRetry={retry} />;
  if (!data || !slots) return <ListSkeleton label="Loading research..." />;
  const shownSlot = shown ? historySlots?.find(slot => slot.index === shown.slotIndex) : undefined;
  // The assignment stays resolved once it completes or is switched out.
  const shownEntry = shownSlot?.entries.find(entry => entry.token === shown!.token
    && (shown!.firstObservedHour === null ? entry.outcome === 'active' : entry.firstObservedHour === shown!.firstObservedHour));
  const card = (item: ActiveSlot) => {
    const key = `slot:${item.slot.index}`;
    const meta = <span className="active-card-meta">Slot {item.slot.index + 1}</span>;
    if (item.kind === 'unavailable') return <div key={key} className="active-card is-quiet" role="status">
      <span className="active-card-art is-icon"><Icon name="clock" /></span>
      <span className="active-card-body">{meta}<b className="active-card-name is-muted">Unavailable</b><small>Not recorded at this hour</small></span></div>;
    if (item.kind === 'idle') return <div key={key} className="active-card is-quiet" role="status">
      <span className="active-card-art is-icon"><Icon name="pause" /></span>
      <span className="active-card-body">{meta}<b className="active-card-name is-idle">Idle</b>
        {item.savedRaw > 0n && <small>{researchNumber(item.savedRaw)} days saved for the next research</small>}</span></div>;
    const { entry, token, progress, left, aheadOf, slot } = item;
    const databaseIndex = entry?.databaseIndex ?? null;
    const name = text.name(token, databaseIndex), look = researchFolderAt(looks, token, databaseIndex), icon = text.icon(token);
    const content = () => {
      const pending = historyPreparing || data === currentData;
      const tip = currentTooltip(token, slot.index, progress, left, !!entry || pending, text, aheadOf, databaseIndex);
      return pending ? { ...tip, note: [tip.note, 'Assignment dates and time breakdown are still being prepared.', countdownPreparing ? 'The carried days-left estimate is still being prepared.' : null].filter(Boolean).join(' ') } : tip;
    };
    const tips = {
      onPointerMove: (event: { clientX: number; clientY: number }) => showTooltip(owner, content(), event.clientX, event.clientY, key),
      onPointerLeave: untip,
      onFocus: (event: { currentTarget: Element }) => { const box = event.currentTarget.getBoundingClientRect(); showTooltip(owner, content(), box.right, box.top, key); },
      onBlur: untip,
    };
    const body = <>
      <span className="active-card-art">{icon ? <img src={icon} alt="" /> : <ArtFallback icon="flask" />}</span>
      <span className="active-card-body">
        <span className="active-card-meta">Slot {slot.index + 1}{look && <><i style={{ background: look.colour }} />{look.name}</>}
          {aheadOf !== null && <span className="pill warning active-card-ahead">Ahead of time</span>}</span>
        <b className="active-card-name">{name}</b>
        <ProgressMeter value={progress?.fraction ?? null} state="live" label={`${name} progress`} />
        <span className="active-card-foot"><b>{progress ? `${Math.round(progress.fraction * 100)}%` : ''}</b>
          <span className="active-card-days">{live && <span className="live-dot" />}{left !== null ? `${left} ${left === 1 ? 'day' : 'days'} left` : 'Researching'}</span></span>
      </span>
    </>;
    return <button key={key} type="button" className="active-card" aria-label={`${name}, researching in slot ${slot.index + 1}, show time breakdown`} {...tips}
      {...rowJumpProps(runtime, track.id, entry ? () => researchJourney(slots, slot, entry).firstHour : null, menu.open)}
      onClick={event => { const target = event.currentTarget; untip(); setShown({ scope, slotIndex: slot.index, token, firstObservedHour: entry?.firstObservedHour ?? null });
        breakdown.openAt(() => target.isConnected ? target.getBoundingClientRect() : null); }}>
      {body}<Icon name="chevron-down" className="active-card-chevron" aria-hidden="true" /></button>;
  };
  return <section key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag])} className="active-research" aria-label="Active research" ref={parity}>
    {header}
    {error && <WidgetState kind="error" failure={error} onRetry={retry} />}
    <div className="active-cards">{cards.map(card)}</div>
    {!cards.length && <p>No research slots recorded at {researchDate(slotHour ?? hour)}.</p>}
    <ResearchGrants inventory={grantsShown} loading={grants !== undefined && grantsShown === undefined} />
    {historyPreparing && <p className="chart-line-key" role="status">Assignment dates and time breakdown are still being prepared.{countdownPreparing && ' The carried days-left estimate is still being prepared.'}</p>}
    <Popover menu={breakdown} className="research-popover" aria-label="Research time breakdown">
      {breakdown.open && <>
        {history.error && history.data ? <WidgetState kind="error" failure={history.error} onRetry={history.retry} /> : null}
        {history.error && !history.data ? <WidgetState kind="error" failure={history.error} onRetry={history.retry} /> : historyPreparing || !history.data ? <PreparedLogCoverage coverage={null} /> : shownSlot && shownEntry ? <>
        <PopoverHeading title={text.name(shownEntry.token, shownEntry.databaseIndex)} menu={breakdown} />
        <ResearchExplanation history={history.data.history} slots={historySlots!} slot={shownSlot} entry={shownEntry}
          description={researchArt(artwork, shownEntry.token, shownEntry.databaseIndex)?.description} />
        </> : <p role="status">Assignment history was not recorded.</p>}
      </>}
    </Popover>
    {menu.menu}
  </section>;
}
