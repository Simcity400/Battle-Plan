import { useGate } from '../../dashboard/regionGate';
import { useRef, useSyncExternalStore } from 'react';
import ArtFallback from '../../components/ArtFallback';
import FloatingPanel from '../../components/FloatingPanel';
import type { FloatingAnchor } from '../../components/FloatingPanel';
import WidgetState from '../../components/WidgetState';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import { useDrawnPanel } from '../../dashboard/DrawnPanel';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { Failure } from '../../dashboard/queryError';
import type { ResearchSlotsReply } from '../../generated/dashboard';
import type { PanelFeed } from '../../dashboard/panelFeed';
import ResearchExplanation from '../research/ResearchExplanation';
import { TechnologyEffects } from '../research/TechnologyWindow';
import { completedBy, researchAt } from '../research/researchData';
import { estimateHour, slotNumbers } from '../research/researchSlots';
import type { ResearchData, ResearchSlotsAt } from '../research/types';
import type { ActiveSlot, OpenedResearch } from './activeResearchData';
import { researchAfter } from './activeResearchData';
import { researchDate } from '../research/researchLayout';
import '../research/research.css';
import '../../components/Dossier.css';

export interface ResearchShown { name: string; icon: string | undefined }

export type ResearchWindowStatus =
  | { kind: 'ready'; reply: ResearchSlotsAt }
  | { kind: 'loading' }
  | { kind: 'outside' }
  | { kind: 'error'; failure: Failure; onRetry: () => void };

export default function ResearchWindow({ runtime, widget, track, tag, data, status, opened, item, shown, feed, installed, subscribe, active, anchor, onClose }: {
  runtime: WidgetRuntime; widget: WidgetSpec; track: BoundTrack; tag: string; data: ResearchData; status: ResearchWindowStatus; opened: OpenedResearch;
  item: Extract<ActiveSlot, { kind: 'researching' }> | null; shown: ResearchShown; feed: PanelFeed | null;
  installed: () => { texts: ReadonlyMap<string, string>; hour: number; data: ResearchSlotsReply | undefined } | undefined; subscribe: (changed: () => void) => () => void;
  active: boolean; anchor: FloatingAnchor; onClose: () => void;
}) {
  const facts = useRef<HTMLDivElement>(null);
  const gate = useGate(runtime, widget);
  const readyReply = status.kind === 'ready' ? status.reply : null;
  const readyItem = readyReply ? item : null;
  useDrawnPanel(facts, runtime.clock, readyItem ? feed : null, active);
  const numbers = readyItem && readyReply && slotNumbers(readyItem.read, readyReply.hour);
  const estimated = useSyncExternalStore(subscribe, () => readyItem && readyReply ? estimateHour(installed(), readyReply, readyItem.slot.index, readyItem.token) : readyReply?.hour ?? 0);
  const slots = readyReply ? researchAt(data.history, readyReply.hour) : [];
  const estimatedSlots = readyReply ? researchAt(data.history, estimated) : [];
  const slot = readyItem && (estimatedSlots.find(owner => owner.index === readyItem.slot.index) ?? readyItem.slot);
  const after = !readyReply || readyItem ? null : researchAfter(slots, opened, readyReply.hour);
  const finished = after?.kind === 'completed' ? after : null;
  return <FloatingPanel title="Research" label="research details" anchor={anchor} onClose={onClose} className="research-window">
    <div className="dossier">
      <header className="dossier-hero">
        <span className="dossier-art">{shown.icon ? <img src={shown.icon} alt="" /> : <ArtFallback icon="flask" />}</span>
        <div className="dossier-title">
          <h3>{shown.name}</h3>
        </div>
      </header>
      {status.kind === 'loading' && <WidgetState kind="loading" scale="section" label="Reading research…" delayMs={0} />}
      {status.kind === 'outside' && <WidgetState kind="outside" scale="section" />}
      {status.kind === 'error' && <WidgetState kind="error" scale="section" what="research" failure={status.failure} onRetry={status.onRetry} />}
      {readyItem && numbers && <div className="dossier-progress" ref={facts} data-research-slot={readyItem.slot.index}>
        <strong className="dossier-progress-value" data-paint="percent">{numbers.percent}</strong>
        <ProgressMeter value={numbers.fraction} state="live" label={`${shown.name} progress`} />
        <span className="dossier-progress-side" data-paint="days">{numbers.days}</span>
      </div>}
      {finished && <div className="dossier-progress is-done"><strong className="dossier-progress-value">Completed {completedBy(finished.entry) ? 'by ' : ''}{researchDate(finished.hour)}</strong></div>}
      {after?.kind === 'gone' && <p className="dossier-missing">{after.message}</p>}
      <div className="dossier-body">
      <TechnologyEffects runtime={runtime} widget={widget} tag={tag} token={opened.token} databaseIndex={opened.databaseIndex} active={active} />
      {readyItem && slot && (readyItem.entry
        ? <section className="dossier-section"><h4>Research time</h4>
          <ResearchExplanation client={runtime.client} gate={gate} track={track} tag={tag} hour={estimated} history={data.history} slots={estimatedSlots} slot={slot} entry={readyItem.entry} />
        </section>
        : <p className="dossier-missing">The research time breakdown is unavailable: this slot's history was not joined.</p>)}
      {finished && readyReply && <section className="dossier-section"><h4>Research time</h4>
        <ResearchExplanation client={runtime.client} gate={gate} track={track} tag={tag} hour={readyReply.hour} history={data.history} slots={slots} slot={finished.slot} entry={finished.entry} />
      </section>}
      </div>
    </div>
  </FloatingPanel>;
}
