import { useRef, useSyncExternalStore } from 'react';
import ArtFallback from '../../components/ArtFallback';
import FloatingPanel from '../../components/FloatingPanel';
import type { FloatingAnchor } from '../../components/FloatingPanel';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import { useDrawnPanel } from '../../dashboard/DrawnPanel';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
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

export default function ResearchWindow({ runtime, widget, track, tag, data, reply, opened, item, shown, feed, installed, subscribe, active, anchor, onClose }: {
  runtime: WidgetRuntime; widget: WidgetSpec; track: BoundTrack; tag: string; data: ResearchData; reply: ResearchSlotsAt; opened: OpenedResearch;
  item: Extract<ActiveSlot, { kind: 'researching' }> | null; shown: ResearchShown; feed: PanelFeed | null;
  installed: () => { texts: ReadonlyMap<string, string>; hour: number; data: ResearchSlotsReply | undefined } | undefined; subscribe: (changed: () => void) => () => void;
  active: boolean; anchor: FloatingAnchor; onClose: () => void;
}) {
  const facts = useRef<HTMLDivElement>(null);
  useDrawnPanel(facts, runtime.clock, item ? feed : null, active);
  const numbers = item && slotNumbers(item.read, reply.hour);
  const estimated = useSyncExternalStore(subscribe, () => item ? estimateHour(installed(), reply, item.slot.index, item.token) : reply.hour);
  const slots = researchAt(data.history, reply.hour);
  const estimatedSlots = researchAt(data.history, estimated);
  const slot = item && (estimatedSlots.find(owner => owner.index === item.slot.index) ?? item.slot);
  const after = item ? null : researchAfter(slots, opened, reply.hour);
  const finished = after?.kind === 'completed' ? after : null;
  return <FloatingPanel title="Research" label="research details" anchor={anchor} onClose={onClose} className="research-window">
    <div className="dossier">
      <header className="dossier-hero">
        <span className="dossier-art">{shown.icon ? <img src={shown.icon} alt="" /> : <ArtFallback icon="flask" />}</span>
        <div className="dossier-title">
          <h3>{shown.name}</h3>
        </div>
      </header>
      {item && numbers && <div className="dossier-progress" ref={facts} data-research-slot={item.slot.index}>
        <strong className="dossier-progress-value" data-paint="percent">{numbers.percent}</strong>
        <ProgressMeter value={numbers.fraction} state="live" label={`${shown.name} progress`} />
        <span className="dossier-progress-side" data-paint="days">{numbers.days}</span>
      </div>}
      {finished && <div className="dossier-progress is-done"><strong className="dossier-progress-value">Completed {completedBy(finished.entry) ? 'by ' : ''}{researchDate(finished.hour)}</strong></div>}
      {after?.kind === 'gone' && <p className="dossier-missing">{after.message}</p>}
      <div className="dossier-body">
      <TechnologyEffects runtime={runtime} widget={widget} tag={tag} token={opened.token} databaseIndex={opened.databaseIndex} active={active} />
      {item && slot && (item.entry
        ? <section className="dossier-section"><h4>Research time</h4>
          <ResearchExplanation client={runtime.client} track={track} tag={tag} hour={estimated} history={data.history} slots={estimatedSlots} slot={slot} entry={item.entry} />
        </section>
        : <p className="dossier-missing">The research time breakdown is unavailable: this slot's history was not joined.</p>)}
      {finished && <section className="dossier-section"><h4>Research time</h4>
        <ResearchExplanation client={runtime.client} track={track} tag={tag} hour={reply.hour} history={data.history} slots={slots} slot={finished.slot} entry={finished.entry} />
      </section>}
      </div>
    </div>
  </FloatingPanel>;
}
