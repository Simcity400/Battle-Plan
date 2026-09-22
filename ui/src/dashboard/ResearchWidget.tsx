import { useEffect, useMemo, useState } from 'react';
import type { ResearchReply } from './client';
import { countryData } from './countryData';
import { researchAt, researchColumns, researchStateAt, researchView } from './researchData';
import { researchNumber } from './researchBreakdown';
import type { ResearchEntry, ResearchSlotHistory, ResearchSlotState } from '../generated/analytics';
import ResearchExplanation from './ResearchExplanation';
import { resolveWindow } from './model';
import type { WidgetSpec } from './model';
import { usePlaybackHour } from './playback';
import type { WidgetRuntime } from './widgets';
import './nationalFocus.css';
import './research.css';

const NO_WARNINGS: string[] = [];
function ResearchCard({ history, slots, slot, entry, current, art }: {
  history: ResearchReply['history'];
  slots: ResearchSlotHistory[]; slot: ResearchSlotHistory; entry: ResearchEntry; current: ResearchSlotState | null;
  art: ResearchReply['artwork'][number] | undefined;
}) {
  const [open, setOpen] = useState(false);
  const isCurrent = current?.token === entry.token && current.entryFirstObservedHour === entry.firstObservedHour;
  return <li><details className="focus-details research-details" onToggle={event => setOpen(event.currentTarget.open)}>
    <summary className={`focus-row research-row${isCurrent ? ' is-current' : ''}`} aria-current={isCurrent ? 'true' : undefined}
      data-tooltip={isCurrent ? 'Assigned at the selected date' : undefined}
      aria-label={`${art?.name ?? entry.token}, slot ${slot.index + 1}${isCurrent ? ', assigned at selected date' : ''}, show details`}>
      <span className="focus-art">{art?.icon ? <img src={art.icon} alt="" width="66" height="64" /> : <span className="focus-missing-art" aria-label="Game artwork unavailable">&#9671;</span>}</span>
      <span className="focus-label"><strong>{art?.name ?? entry.token}</strong>
        {isCurrent && entry.outcome === 'active' && <span className="research-live-eta" data-tooltip="Estimated at the selected hour's research rate.">
          {current?.details?.remainingDays != null ? `≈ ${current.details.remainingDays} days left` : 'Remaining unavailable'}
        </span>}
      </span>
    </summary>
    {open && <ResearchExplanation history={history} slots={slots} slot={slot} entry={entry} current={isCurrent ? current : null} description={art?.description} />}
  </details></li>;
}

export default function ResearchWidget({ widget, runtime, active = true, onDiagnostics }: {
  widget: WidgetSpec; runtime: WidgetRuntime; active?: boolean; onDiagnostics?: (warnings: string[]) => void;
}) {
  usePlaybackHour(runtime.clock, active && widget.time.mode === 'follow');
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const tag = widget.countries.kind === 'tags' ? widget.countries.tags[0] : track?.selectedTag
    ?? track?.source.metadata.countries.find(c => c.id === track.source.metadata.playerCountryId)?.tag;
  const window = track && resolveWindow(widget, track, runtime.tracks, runtime.clock.getHour());
  const hour = window?.lastHour;
  const live = widget.time.mode === 'follow';
  const sourceId = track?.source.id, revision = track?.source.revision, branchId = track?.branch.id;
  const key = JSON.stringify([sourceId, revision, branchId, tag]);
  const [state, setState] = useState<{ key: string; data?: ResearchReply; error?: string }>();
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    if (!active || !sourceId || !revision || !branchId || !tag || !runtime.client.researchHistory) return;
    const controller = new AbortController();
    void countryData(runtime.client).research({ sourceId, revision, branchId, tag }, controller.signal).then(
      data => { if (!controller.signal.aborted) setState({ key, data }); },
      error => { if (!controller.signal.aborted) setState({ key, error: String(error instanceof Error ? error.message : error) }); },
    );
    return () => controller.abort();
  }, [active, sourceId, revision, branchId, tag, runtime.client, key, retry]);
  const data = state?.key === key ? state.data : undefined;
  const error = state?.key === key ? state.error : undefined;
  useEffect(() => { onDiagnostics?.(data?.warnings ?? NO_WARNINGS); return () => onDiagnostics?.(NO_WARNINGS); }, [data, onDiagnostics]);
  const artwork = useMemo(() => new Map(data?.artwork.map(a => [a.token, a])), [data]);
  const slots = useMemo(() => data && hour != null ? researchAt(data.history, hour) : null, [data, hour]);
  const columns = useMemo(() => slots && researchColumns(researchView(slots)), [slots]);
  if (!track || !tag) return <div className="dash-empty" role="status">Load a campaign and choose a country to see its research.</div>;
  if (!runtime.client.researchHistory) return <div className="dash-empty" role="status">Update the app to read research history.</div>;
  if (hour == null || hour < track.branch.firstHour || hour > track.branch.lastHour) return <div className="dash-empty" role="status">This time is outside the country’s recorded branch.</div>;
  if (error) return <div className="dash-empty" role="alert"><p>{error}</p><button className="button small" onClick={() => { countryData(runtime.client).retryResearch(track.source.id); setState(undefined); setRetry(n => n + 1); }}>Retry</button></div>;
  if (!data) return <div className="dash-empty" role="status">Loading research…</div>;
  if (!columns || !slots) return <div className="dash-empty" role="status">No complete research data recorded at this date.</div>;
  if (!columns.length) return <div className="dash-empty" role="status">No research slots recorded at this date.</div>;
  return <section key={key} className="research-widget" aria-label="Research slot history">
    <div className="research-scroll">
      <div className="research-columns" style={{ minWidth: columns.length * 190 }}>
        {columns.map(({ slot, offset }) => {
          const current = live ? researchStateAt(slot, hour) : null;
          const currentEntry = current && slots.find(owner => owner.index === slot.index)?.entries.find(entry => entry.outcome === 'active'
            && entry.firstObservedHour === current.entryFirstObservedHour && entry.token === current.token);
          const earlier = slot.entries;
          return <section className="research-column" key={slot.index} aria-label={`Slot ${slot.index + 1}`}>
          <h3 className="research-column-heading">Slot {slot.index + 1}{live && <span className="research-slot-status"
            data-tooltip={current?.token ? `Assigned: ${artwork.get(current.token)?.name ?? current.token}` : undefined}>
            {!current ? 'Unavailable' : current.token === null
              ? `Idle${BigInt(current.bankedDaysRaw) > 0n ? ` · ${researchNumber(BigInt(current.bankedDaysRaw))} d saved` : ''}` : 'Assigned'}</span>}</h3>
          {currentEntry && <ol className="research-entries research-current-assignment">
            <ResearchCard history={data.history} slots={slots} slot={slot} entry={currentEntry} current={current} art={artwork.get(currentEntry.token)} />
          </ol>}
          {live && earlier.length > 0 && <div className="research-earlier-label">Completed research</div>}
          <ol className="research-entries" style={{ paddingTop: `calc(${offset} * var(--research-row-height))` }}>
          {earlier.map(entry => <ResearchCard key={`${entry.firstObservedHour}:${entry.token}`} history={data.history} slots={slots} slot={slot} entry={entry}
            current={current} art={artwork.get(entry.token)} />)}
          </ol>
          {!live && earlier.length === 0 && <div className="research-empty-slot">No completed research</div>}
        </section>; })}
      </div>
    </div>
  </section>;
}
