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
import { hideTooltip, showTooltip } from '../../charts/tooltip';
import { useChartTokens } from '../../charts/useChartTokens';
import { contentTitle } from '../../dashboard/contentTitles';
import type { WidgetProps } from '../../dashboard/definitions';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import ResearchExplanation from '../research/ResearchExplanation';
import { ResearchWithDoctrines } from '../research/DoctrinePanel';
import { researchArt, researchArtwork, researchAt, researchFolderAt, researchFolderLooks, researchLatestProgress, researchRecordedTemplates } from '../research/researchData';
import { researchJourney } from '../research/researchJourney';
import { currentTooltip, researchDate } from '../research/researchLayout';
import type { ResearchText } from '../research/researchLayout';
import { slotNumbers, slotsStructure } from '../research/researchSlots';
import { historyTokens, useResearchReply, useResearchSlots } from '../research/useResearch';
import { useTechnologyArtwork } from '../research/useTechnologyArtwork';
import { activeSlots } from './activeResearchData';
import ResearchGrants from './ResearchGrantInventory';
import type { ActiveSlot } from './activeResearchData';
import '../research/research.css';
import './activeResearch.css';

const NO_WARNINGS: string[] = [];
const NO_TOKENS: readonly string[] = [];

/** One card per research slot open at the hour shown: what it researches, its
 * latest read progress, the game's estimate and idle slots' saved days. */
export default function ActiveResearchWidget(props: WidgetProps) {
  return <ResearchWithDoctrines {...props} body={ActiveResearchBody} />;
}

/** An opened breakdown: the slot, the assignment's first observation, and the hour it was opened at. */
type Shown = { scope: string; slotIndex: number; token: string; firstObservedHour: number; hour: number };

function ActiveResearchBody({ widget, runtime, active, onDiagnostics, headerSlot }: WidgetProps) {
  const menu = useContextMenu('Active research actions');
  const owner = useId();
  const live = widget.time.mode === 'follow';
  const history = useResearchReply(widget, runtime, active);
  const { track, tag, data } = history;
  const root = useRef<HTMLElement | null>(null);
  // Cards render when what the slots hold changes; their numbers are painted each frame (progress from live cells).
  const now = useResearchSlots(widget, runtime, active && !!data, slotsStructure, root);
  const { latest } = now;
  const reply = now.data;
  const { categories } = useChartTokens();
  const cards = useMemo(() => data && reply ? activeSlots(data.history, reply) : [], [data, reply]);
  const looks = useMemo(() => data ? researchFolderLooks(data, historyTokens(data.history), categories) : new Map(), [data, categories]);
  // Every technology the country's history names, asked once with the history as Research history asks it (one store):
  // a slot taking one up while playing asks nothing.
  const tokens = useMemo(() => data ? [...new Set(historyTokens(data.history).map(use => typeof use === 'string' ? use : use[0]))] : NO_TOKENS, [data]);
  const recorded = useMemo(() => researchRecordedTemplates(data?.history), [data]);
  const images = useTechnologyArtwork(runtime.client, track, tokens, active && !!data, recorded);
  const diagnostics = useMemo(() => [...new Set([...(data?.warnings ?? NO_WARNINGS), ...(reply?.warnings ?? NO_WARNINGS), ...images.warnings])], [data, reply, images.warnings]);
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
  const [stored, setShown] = useState<Shown | null>(null);
  const shown = stored?.scope === scope ? stored : null;
  const { close } = breakdown;
  useEffect(() => close(), [close, scope]);
  const researching = cards.filter(card => card.kind === 'researching').length;
  const header = headerSlot && cards.length > 0 && createPortal(<span className="pill">{researching} of {cards.length} researching</span>, headerSlot);
  if (!track || !tag) return <div className="dash-empty" role="status"><span className="empty-icon"><Icon name="flask" /></span><p>Load a campaign and choose a country to see its research.</p></div>;
  if (now.read === 'off') return <div className="dash-empty" role="status">This time is outside the recording.</div>;
  const failure = history.error ?? now.error;
  if (failure && (!data || !reply)) return <WidgetState kind="error" failure={failure} onRetry={history.error ? history.retry : now.retry} />;
  if (!data || !reply) return <ListSkeleton label="Loading research..." />;
  const detailSlots = shown ? researchAt(data.history, shown.hour) : null;
  const shownSlot = shown ? detailSlots?.find(slot => slot.index === shown.slotIndex) : undefined;
  const shownEntry = shownSlot?.entries.find(entry => entry.token === shown!.token && entry.firstObservedHour === shown!.firstObservedHour);
  const card = (item: ActiveSlot) => {
    const key = `slot:${item.slot.index}`;
    const meta = <span className="active-card-meta">Slot {item.slot.index + 1}</span>;
    if (item.kind === 'unavailable') return <div key={key} className="active-card is-quiet" role="status">
      <span className="active-card-art is-icon"><Icon name="clock" /></span>
      <span className="active-card-body">{meta}<b className="active-card-name is-muted">Unavailable</b><small>{item.reason}</small></span></div>;
    const numbers = slotNumbers(item.read, reply.hour);
    if (item.kind === 'idle') return <div key={key} className="active-card is-quiet" role="status" data-research-slot={item.slot.index}>
      <span className="active-card-art is-icon"><Icon name="pause" /></span>
      <span className="active-card-body">{meta}<b className="active-card-name is-idle">Idle</b>
        {numbers.saved && <small><span data-paint="saved">{numbers.saved}</span> for the next research</small>}</span></div>;
    const { entry, token, aheadOf, slot } = item;
    const databaseIndex = entry?.databaseIndex ?? null;
    const name = text.name(token, databaseIndex), look = researchFolderAt(looks, token, databaseIndex), icon = text.icon(token);
    const content = () => {
      const at = latest.current ?? reply, hour = at.hour, current = at.slots.find(read => read.index === slot.index) ?? item.read;
      const full = data.history.slots.find(owner => owner.index === slot.index) ?? slot;
      const progress = researchLatestProgress({ ...full, states: [current.state], progress: current.progress ? [current.progress] : [] }, current.state, hour);
      return currentTooltip(token, slot.index, progress, slotNumbers(current, hour).left, !!entry, text, aheadOf, databaseIndex);
    };
    const tips = {
      onPointerMove: (event: { clientX: number; clientY: number }) => showTooltip(owner, content(), event.clientX, event.clientY, key),
      onPointerLeave: untip,
      onFocus: (event: { currentTarget: Element }) => { const box = event.currentTarget.getBoundingClientRect(); showTooltip(owner, content(), box.right, box.top, key); },
      onBlur: untip,
    };
    const first = () => {
      if (!entry) return null;
      const slots = researchAt(data.history, now.hourNow()), at = slots.find(owner => owner.index === slot.index);
      return at ? researchJourney(slots, at, entry).firstHour : null;
    };
    const body = <>
      <span className="active-card-art">{icon ? <img src={icon} alt="" /> : <ArtFallback icon="flask" />}</span>
      <span className="active-card-body">
        <span className="active-card-meta">Slot {slot.index + 1}{look && <><i style={{ background: look.colour }} />{look.name}</>}
          {aheadOf !== null && <span className="pill warning active-card-ahead">Ahead of time</span>}</span>
        <b className="active-card-name">{name}</b>
        <ProgressMeter value={numbers.fraction} state="live" label={`${name} progress`} />
        <span className="active-card-foot"><b data-paint="percent">{numbers.percent}</b>
          <span className="active-card-days">{live && <span className="live-dot" />}<span data-paint="days">{numbers.days}</span></span></span>
      </span>
    </>;
    if (!entry) return <div key={key} className="active-card" data-research-slot={slot.index} tabIndex={0} aria-label={`${name}, researching in slot ${slot.index + 1}`} {...tips}>{body}</div>;
    return <button key={key} type="button" className="active-card" data-research-slot={slot.index} aria-label={`${name}, researching in slot ${slot.index + 1}, show time breakdown`} {...tips}
      {...rowJumpProps(runtime, track.id, first, menu.open)}
      onClick={event => { const target = event.currentTarget; untip(); setShown({ scope, slotIndex: slot.index, token, firstObservedHour: entry.firstObservedHour, hour: now.hourNow() });
        breakdown.openAt(() => target.isConnected ? target.getBoundingClientRect() : null); }}>
      {body}<Icon name="chevron-down" className="active-card-chevron" aria-hidden="true" /></button>;
  };
  return <section key={scope} className="active-research" aria-label="Active research" ref={now.root}>
    {header}
    {failure && <WidgetState kind="error" failure={failure} onRetry={history.error ? history.retry : now.retry} />}
    <div className="active-cards">{cards.map(card)}</div>
    {!cards.length && <p>{reply.missing ?? `No research slots recorded at ${researchDate(reply.hour)}.`}</p>}
    <ResearchGrants inventory={reply.grants} />
    <Popover menu={breakdown} className="research-popover" aria-label="Research time breakdown">
      {breakdown.open && (shownSlot && shownEntry && detailSlots ? <>
        <PopoverHeading title={text.name(shownEntry.token, shownEntry.databaseIndex)} menu={breakdown} />
        <ResearchExplanation client={runtime.client} track={track} tag={tag} hour={shown!.hour} history={data.history} slots={detailSlots} slot={shownSlot} entry={shownEntry}
          description={researchArt(artwork, shownEntry.token, shownEntry.databaseIndex)?.description} />
      </> : <p role="status">Assignment history was not recorded.</p>)}
    </Popover>
    {menu.menu}
  </section>;
}
