import { useEffect, useMemo, useState } from 'react';
import type { CSSProperties } from 'react';
import type { WidgetProps } from '../../dashboard/definitions';
import { defaultCountryTag } from '../../dashboard/countrySelection';
import { resolveWindow } from '../../dashboard/model';
import { usePlaybackHour } from '../../dashboard/playback';
import { useProduct } from '../../dashboard/products';
import type { ResearchEntry, ResearchReply, ResearchSlotHistory, ResearchSlotState } from '../../generated/dashboard';
import ResearchExplanation from './ResearchExplanation';
import { researchNumber } from './researchBreakdown';
import { researchAt, researchColumns, researchProgressAt, researchRequest, researchStateAt, researchView, validateResearchReply } from './researchData';
import '../national-focus/nationalFocus.css';
import ListSkeleton from '../../components/Skeleton';
import { Icon } from '../../components/Icons';
import './research.css';

const NO_WARNINGS: string[] = [];
type Art = ResearchReply['artwork'][number] | undefined;
function ResearchArt({ art }: { art: Art }) {
  return <span className="focus-art is-large">{art?.icon ? <img src={art.icon} alt="" /> : <span className="focus-missing-art" data-tooltip="Game artwork unavailable in the recorded content"><Icon name="flask" /></span>}</span>;
}

/** A completed technology. Its row opens the full explanation. */
function ResearchCard({ history, slots, slot, entry, art }: {
  history: ResearchReply['history']; slots: ResearchSlotHistory[]; slot: ResearchSlotHistory; entry: ResearchEntry; art: Art;
}) {
  const [open, setOpen] = useState(false);
  return <li><details className="focus-details research-details" onToggle={event => setOpen(event.currentTarget.open)}>
    <summary className="focus-row research-row" aria-label={`${art?.name ?? entry.token}, slot ${slot.index + 1}, show details`}>
      <span className="focus-art">{art?.icon ? <img src={art.icon} alt="" width="66" height="64" /> : <span className="focus-missing-art" aria-label="Game artwork unavailable">&#9671;</span>}</span>
      <span className="focus-label"><strong title={art?.name ?? entry.token}>{art?.name ?? entry.token}</strong></span>
      <Icon name="chevron-down" className="focus-chevron" aria-hidden="true" />
    </summary>
    {open && <ResearchExplanation history={history} slots={slots} slot={slot} entry={entry} description={art?.description} />}
  </details></li>;
}

/** Live or pinned: the slot's assignment at that hour, with the game's
 * progress line along its lower edge. Clicking it opens the explanation. */
function CurrentResearch({ history, slots, slot, entry, current, art, hour, live }: {
  history: ResearchReply['history']; slots: ResearchSlotHistory[]; slot: ResearchSlotHistory; entry: ResearchEntry;
  current: ResearchSlotState; art: Art; hour: number; live: boolean;
}) {
  const [open, setOpen] = useState(false);
  const progress = researchProgressAt(slot, entry.firstObservedHour, hour);
  const left = current.details?.remainingDays ?? null;
  const name = art?.name ?? entry.token;
  return <details className="research-now-details" onToggle={event => setOpen(event.currentTarget.open)}>
    <summary className="focus-now research-now" aria-label={`${name}, researching in slot ${slot.index + 1}, show details`}>
      <ResearchArt art={art} />
      <span className="focus-now-text"><strong title={name}>{name}</strong>
        <span className="focus-now-status" data-tooltip={left !== null ? 'The game’s estimate at this hour’s research speed' : undefined}>{live && <span className="live-dot" />}
          {left !== null ? `${left} ${left === 1 ? 'day' : 'days'} left` : progress !== null ? `${Math.round(progress * 100)}% complete` : 'Researching'}</span></span>
      <Icon name="chevron-down" className="focus-chevron" aria-hidden="true" />
      {progress !== null && <span className="focus-progress" role="progressbar" aria-label={`${name} progress`} aria-valuemin={0} aria-valuemax={100}
        aria-valuenow={Math.round(progress * 100)} style={{ '--focus-progress': progress } as CSSProperties}><span /></span>}
    </summary>
    {open && <ResearchExplanation history={history} slots={slots} slot={slot} entry={entry} description={art?.description} />}
  </details>;
}

function IdleSlot({ current }: { current: ResearchSlotState | null }) {
  const banked = current ? BigInt(current.bankedDaysRaw) : 0n;
  return <div className="focus-now is-idle research-now" role="status">
    <span className="focus-art is-large"><span className="focus-missing-art"><Icon name="pause" /></span></span>
    <span className="focus-now-text"><strong>{current ? 'Idle' : 'Unavailable'}</strong>
      <span>{!current ? 'Not recorded at this hour' : banked > 0n ? `${researchNumber(banked)} days saved for the next research` : 'Nothing assigned to this slot'}</span></span>
  </div>;
}

export default function ResearchWidget({ widget, runtime, active, onDiagnostics }: WidgetProps) {
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
  useEffect(() => { onDiagnostics(data?.warnings ?? NO_WARNINGS); return () => onDiagnostics(NO_WARNINGS); }, [data, onDiagnostics]);
  const artwork = useMemo(() => new Map(data?.artwork.map(a => [a.token, a])), [data]);
  const slots = useMemo(() => data && hour != null ? researchAt(data.history, hour) : null, [data, hour]);
  const columns = useMemo(() => slots && researchColumns(researchView(slots)), [slots]);
  if (!track || !tag) return <div className="dash-empty" role="status"><span className="empty-icon"><Icon name="flask" /></span><p>Load a campaign and choose a country to see its research.</p></div>;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <div className="dash-empty" role="status">This time is outside the country’s recorded branch.</div>;
  if (error) return <div className="dash-empty" role="alert"><p>{error}</p><button className="button small" onClick={retry}>Retry</button></div>;
  if (!data) return <ListSkeleton label="Loading research…" />;
  if (!columns || !slots) return <div className="dash-empty" role="status">No complete research data recorded at this date.</div>;
  if (!columns.length) return <div className="dash-empty" role="status">No research slots recorded at this date.</div>;
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
            ? <CurrentResearch history={data.history} slots={slots} slot={slot} entry={currentEntry} current={current} art={artwork.get(currentEntry.token)} hour={hour} live={live} />
            : <IdleSlot current={current} />)}
          {moment && earlier.length > 0 && <div className="research-earlier-label">Completed</div>}
          <ol className="research-entries" style={{ paddingTop: `calc(${offset} * var(--research-row-height))` }}>
          {earlier.map(entry => <ResearchCard key={`${entry.firstObservedHour}:${entry.token}`} history={data.history} slots={slots} slot={slot} entry={entry}
            art={artwork.get(entry.token)} />)}
          </ol>
          {!moment && earlier.length === 0 && <div className="research-empty-slot">No completed research</div>}
        </section>; })}
      </div>
    </div>
  </section>;
}
