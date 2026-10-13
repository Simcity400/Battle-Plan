import ArtFallback from '../../components/ArtFallback';
import { useMemo, useState } from 'react';
import type { CSSProperties } from 'react';
import type { WidgetProps } from '../../dashboard/definitions';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles';
import { useDiagnostics } from '../../dashboard/useDiagnostics';
import { useTechnologyArtwork } from './useTechnologyArtwork';
import type { ResearchEntry, ResearchReply, ResearchSlotHistory, ResearchSlotState } from '../../generated/dashboard';
import ResearchExplanation from './ResearchExplanation';
import { researchNumber } from './researchBreakdown';
import { researchJourney } from './researchJourney';
import { isUnjoinedResearch, researchAt, researchArtworkTokens, researchColumns, researchProgressAt, researchRequest, researchStateAt, researchView, validateResearchReply } from './researchData';
import '../national-focus/nationalFocus.css';
import WidgetState from '../../components/WidgetState';
import ListSkeleton from '../../components/Skeleton';
import { Icon } from '../../components/Icons';
import { useContextMenu } from '../../components/ContextMenu';
import { rowJumpProps } from '../../charts/jump';
import './research.css';

const NO_WARNINGS: string[] = [];
type Art = ResearchReply['artwork'][number] | undefined;
function ResearchArt({ art }: { art: Art }) {
  return <span className="focus-art is-large">{art?.icon ? <img src={art.icon} alt="" /> : <ArtFallback icon="flask" />}</span>;
}

/** A completed technology. Its row opens the full explanation. */
function ResearchCard({ history, slots, slot, entry, art, jump }: {
  history: ResearchReply['history']; slots: ResearchSlotHistory[]; slot: ResearchSlotHistory; entry: ResearchEntry; art: Art;
  jump: ReturnType<typeof rowJumpProps>;
}) {
  const [open, setOpen] = useState(false);
  return <li {...jump}><details className="focus-details research-details" onToggle={event => setOpen(event.currentTarget.open)}>
    <summary className="focus-row research-row" aria-label={`${contentTitle(art?.name, entry.token, 'Technology name unavailable')}, slot ${slot.index + 1}, show details`}>
      <span className="focus-art">{art?.icon ? <img src={art.icon} alt="" width="66" height="64" /> : <ArtFallback icon="flask" />}</span>
      <span className="focus-label"><strong data-tooltip={contentTitleHint(entry.token)}>{contentTitle(art?.name, entry.token, 'Technology name unavailable')}</strong></span>
      <Icon name="chevron-down" className="focus-chevron" aria-hidden="true" />
    </summary>
    {open && <ResearchExplanation history={history} slots={slots} slot={slot} entry={entry} description={art?.description} />}
  </details></li>;
}

/** Live or pinned: the slot's assignment at that hour, with the game's
 * progress line along its lower edge. Clicking it opens the explanation. */
function CurrentResearch({ history, slots, slot, entry, current, art, hour, live, jump }: {
  history: ResearchReply['history']; slots: ResearchSlotHistory[]; slot: ResearchSlotHistory; entry: ResearchEntry;
  current: ResearchSlotState; art: Art; hour: number; live: boolean;
  jump: ReturnType<typeof rowJumpProps>;
}) {
  const [open, setOpen] = useState(false);
  const progress = researchProgressAt(slot, entry.firstObservedHour, hour);
  const left = current.details?.remainingDays ?? null;
  const name = contentTitle(art?.name, entry.token, 'Technology name unavailable');
  return <details className="research-now-details" onToggle={event => setOpen(event.currentTarget.open)} {...jump}>
    <summary className="focus-now research-now" aria-label={`${name}, researching in slot ${slot.index + 1}, show details`}>
      <ResearchArt art={art} />
      <span className="focus-now-text"><strong data-tooltip={contentTitleHint(entry.token)}>{name}</strong>
        <span className="focus-now-status" data-tooltip={left !== null ? 'The game’s estimate at this hour’s research speed' : undefined}>{live && <span className="live-dot" />}
          {left !== null ? `${left} ${left === 1 ? 'day' : 'days'} left` : progress !== null ? `${Math.round(progress * 100)}% complete` : 'Researching'}</span></span>
      <Icon name="chevron-down" className="focus-chevron" aria-hidden="true" />
      {progress !== null && <span className="focus-progress" role="progressbar" aria-label={`${name} progress`} aria-valuemin={0} aria-valuemax={100}
        aria-valuenow={Math.round(progress * 100)} style={{ '--focus-progress': progress } as CSSProperties}><span /></span>}
    </summary>
    {open && <ResearchExplanation history={history} slots={slots} slot={slot} entry={entry} description={art?.description} />}
  </details>;
}

/** Live or pinned: a technology the slot holds that no recorded assignment
 * joins (its occurrence was not recorded). Its name and the game's estimate
 * are known; its progress and history are not, so it has no details. */
function UnjoinedResearch({ current, art, live }: { current: ResearchSlotState & { token: string }; art: Art; live: boolean }) {
  const left = current.details?.remainingDays ?? null;
  const name = contentTitle(art?.name, current.token, 'Technology name unavailable');
  return <div className="focus-now research-now" role="status" aria-label={`${name}, researching`}>
    <ResearchArt art={art} />
    <span className="focus-now-text"><strong data-tooltip={contentTitleHint(current.token)}>{name}</strong>
      <span className="focus-now-status" data-tooltip={left !== null ? 'The game’s estimate at this hour’s research speed' : undefined}>{live && <span className="live-dot" />}
        {left !== null ? `${left} ${left === 1 ? 'day' : 'days'} left` : 'Researching'}</span></span>
  </div>;
}

function IdleSlot({ current }: { current: ResearchSlotState | null }) {
  const banked = current ? BigInt(current.bankedDaysRaw) : 0n;
  return <div className="focus-now is-idle research-now" role="status">
    <span className="focus-art is-large"><span className="focus-missing-art"><Icon name="pause" /></span></span>
    <span className="focus-now-text"><strong>{current ? 'Idle' : 'Unavailable'}</strong>
      <span>{!current ? 'Not recorded at this hour' : banked > 0n ? `${researchNumber(banked)} days saved for the next research` : null}</span></span>
  </div>;
}

export default function ResearchWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
  const menu = useContextMenu('Research actions');
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : defaultCountryTag(track) ?? undefined;
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const live = widget.time.mode === 'follow';
  // A pinned moment shows each slot's assignment then, like Live but held still.
  const moment = live || widget.time.mode === 'pin';
  const request = track && tag ? researchRequest(track.source.id, track.source.revision, { branchId: track.branch.id, tag }) : null;
  const { data, error, retry } = useProduct<ResearchReply>(runtime.client, request, active, validateResearchReply);
  const slots = useMemo(() => data && hour != null ? researchAt(data.history, hour) : null, [data, hour]);
  const columns = useMemo(() => slots && researchColumns(researchView(slots)), [slots]);
  const imageTokens = useMemo(() => researchArtworkTokens(slots, hour, moment), [slots, hour, moment]);
  const images = useTechnologyArtwork(runtime.client, track, imageTokens, active && !!data);
  const diagnostics = useMemo(() => [...new Set([...(data?.warnings ?? NO_WARNINGS), ...images.warnings])], [data, images.warnings]);
  useDiagnostics(onDiagnostics, diagnostics);
  const artwork = useMemo(() => new Map(data?.artwork.map(art => [art.token, { ...art, icon: images.artwork.get(art.token)?.icon ?? art.icon }])), [data, images.artwork]);
  if (!track || !tag) return <div className="dash-empty" role="status"><span className="empty-icon"><Icon name="flask" /></span><p>Load a campaign and choose a country to see its research.</p></div>;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <div className="dash-empty" role="status">This time is outside the recording.</div>;
  if (error) return <WidgetState kind="error" detail={error} onRetry={retry} />;
  if (!data) return <ListSkeleton label="Loading research…" />;
  if (!columns || !slots || !columns.length) return <div className="dash-empty" role="status">No research slots recorded by this date.</div>;
  return <section key={JSON.stringify([track.source.id, track.source.revision, track.branch.id, tag])} className="research-widget" aria-label="Research slot history">
    <div className="research-scroll">
      <div className="research-columns" style={{ minWidth: columns.length * 190 }}>
        {columns.map(({ slot, offset }) => {
          const current = moment ? researchStateAt(slot, hour) : null;
          const currentEntry = current && slots.find(owner => owner.index === slot.index)?.entries.find(entry => entry.outcome === 'active'
            && entry.firstObservedHour === current.entryFirstObservedHour && entry.token === current.token);
          const earlier = slot.entries;
          return <section className="research-column" key={slot.index} aria-label={`Slot ${slot.index + 1}`}>
          <h3 className="research-column-heading">Slot {slot.index + 1}</h3>
          {moment && (currentEntry && current
            ? <CurrentResearch history={data.history} slots={slots} slot={slot} entry={currentEntry} current={current} art={artwork.get(currentEntry.token)} hour={hour} live={live}
                jump={rowJumpProps(runtime, track.id, () => researchJourney(slots, slot, currentEntry).firstHour, menu.open)} />
            : isUnjoinedResearch(current) ? <UnjoinedResearch current={current} art={artwork.get(current.token)} live={live} />
            : <IdleSlot current={current} />)}
          <ol className="research-entries" style={{ paddingTop: `calc(${offset} * var(--research-row-height))` }}>
          {earlier.map(entry => <ResearchCard key={`${entry.firstObservedHour}:${entry.token}`} history={data.history} slots={slots} slot={slot} entry={entry}
            art={artwork.get(entry.token)} jump={rowJumpProps(runtime, track.id, entry.endedByHour, menu.open)} />)}
          </ol>
          {!moment && earlier.length === 0 && <div className="research-empty-slot">
            {slot.states.some(isUnjoinedResearch) ? 'Completed research not recorded' : 'No completed research'}</div>}
        </section>; })}
      </div>
    </div>
    {menu.menu}
  </section>;
}
