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
import { hideTooltip, showTooltip } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { useParityRows } from '../../dashboard/parityRows';
import { contentTitle } from '../../dashboard/contentTitles';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackHour } from '../../dashboard/playback';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import type { ResearchEntry } from '../../generated/dashboard';
import ResearchExplanation from '../research/ResearchExplanation';
import { researchNumber } from '../research/researchBreakdown';
import { researchAt, researchFolderLooks } from '../research/researchData';
import { researchJourney } from '../research/researchJourney';
import { coverageAt, currentTooltip, researchDate } from '../research/researchLayout';
import type { ResearchText } from '../research/researchLayout';
import { historyTokens, useResearchReply } from '../research/useResearch';
import { useTechnologyArtwork } from '../research/useTechnologyArtwork';
import { activeSlots } from './activeResearchData';
import type { ActiveSlot } from './activeResearchData';
import '../national-focus/nationalFocus.css';
import '../research/research.css';
import './activeResearch.css';

const NO_WARNINGS: string[] = [];

/** One card per research slot open at the hour shown: what it researches, its
 * latest read progress, the game's estimate and idle slots' saved days. */
export default function ActiveResearchWidget({ widget, runtime, active, onDiagnostics, headerSlot }: WidgetProps) {
  const parity = useRef<HTMLElement>(null);
  const menu = useContextMenu('Active research actions');
  const owner = useId();
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const live = widget.time.mode === 'follow';
  const { track, tag, data, error, retry, preparing } = useResearchReply(widget, runtime, active);
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const { categories } = useChartTokens();
  const slots = useMemo(() => data && hour != null ? researchAt(data.history, hour) : null, [data, hour]);
  const cards = useMemo(() => slots && data && hour != null ? activeSlots(slots, coverageAt(data.history.coverage, hour), hour) : [], [slots, data, hour]);
  if (import.meta.env.MODE === 'parity') useParityRows(parity, () => ({ kind: 'windowed', label: 'Active research', complete: true, total: cards.length,
    rows: cards.map((card, index) => ({ index, key: `slot:${card.slot.index}`, item: card.kind === 'idle' ? { ...card, savedRaw: card.savedRaw.toString() } : card })) }));
  const looks = useMemo(() => data ? researchFolderLooks(data, historyTokens(data.history), categories) : new Map(), [data, categories]);
  const tokens = useMemo(() => [...new Set(cards.flatMap(card => card.kind === 'researching' ? [card.token] : []))], [cards]);
  const images = useTechnologyArtwork(runtime.client, track, tokens, active && !!data);
  const diagnostics = useMemo(() => [...new Set([...(data?.warnings ?? NO_WARNINGS), ...images.warnings])], [data, images.warnings]);
  useDiagnostics(onDiagnostics, diagnostics);
  const artwork = useMemo(() => new Map(data?.artwork.map(art => [art.token, { ...art, icon: images.artwork.get(art.token)?.icon ?? art.icon }])), [data, images.artwork]);
  const text = useMemo<ResearchText>(() => ({
    name: token => contentTitle(artwork.get(token)?.name, token, 'Technology name unavailable'),
    icon: token => artwork.get(token)?.icon ?? undefined,
    folder: token => looks.get(token),
  }), [artwork, looks]);
  const untip = useCallback(() => hideTooltip(owner), [owner]);
  useEffect(() => untip, [untip]);
  const breakdown = usePopover('start');
  const [shown, setShown] = useState<{ slotIndex: number; entry: ResearchEntry } | null>(null);
  const researching = cards.filter(card => card.kind === 'researching').length;
  const header = headerSlot && cards.length > 0 && createPortal(<span className="pill">{researching} of {cards.length} researching</span>, headerSlot);
  if (!track || !tag) return <div className="dash-empty" role="status"><span className="empty-icon"><Icon name="flask" /></span><p>Load a campaign and choose a country to see its research.</p></div>;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <div className="dash-empty" role="status">This time is outside the recording.</div>;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data || !slots) return preparing ? <PreparedLogCoverage coverage={null} /> : <ListSkeleton label="Loading research…" />;
  if (!cards.length) return <div className="dash-empty" role="status"><span className="empty-icon"><Icon name="flask" /></span>
    <p>No research slots recorded at {researchDate(hour)}.</p></div>;
  const shownSlot = shown ? slots.find(slot => slot.index === shown.slotIndex) : undefined;
  const shownEntry = shownSlot?.entries.find(entry => entry.firstObservedHour === shown!.entry.firstObservedHour && entry.token === shown!.entry.token);
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
    const name = text.name(token), look = looks.get(token), icon = text.icon(token);
    const content = () => currentTooltip(token, slot.index, progress, left, !!entry, text, aheadOf);
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
    if (!entry) return <div key={key} className="active-card" tabIndex={0} aria-label={`${name}, researching in slot ${slot.index + 1}`} {...tips}>{body}</div>;
    return <button key={key} type="button" className="active-card" aria-label={`${name}, researching in slot ${slot.index + 1}, show time breakdown`} {...tips}
      {...rowJumpProps(runtime, track.id, () => researchJourney(slots, slot, entry).firstHour, menu.open)}
      onClick={event => { const target = event.currentTarget; untip(); setShown({ slotIndex: slot.index, entry });
        breakdown.openAt(() => target.isConnected ? target.getBoundingClientRect() : null); }}>
      {body}<Icon name="chevron-down" className="active-card-chevron" aria-hidden="true" /></button>;
  };
  return <section key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag])} className="active-research" aria-label="Active research" ref={parity}>
    {header}
    <div className="active-cards">{cards.map(card)}</div>
    <Popover menu={breakdown} className="research-popover" aria-label="Research time breakdown">
      {breakdown.open && shownSlot && shownEntry && <>
        <PopoverHeading title={text.name(shownEntry.token)} menu={breakdown} />
        <ResearchExplanation history={data.history} slots={slots} slot={shownSlot} entry={shownEntry} description={artwork.get(shownEntry.token)?.description} />
      </>}
    </Popover>
    {menu.menu}
  </section>;
}
