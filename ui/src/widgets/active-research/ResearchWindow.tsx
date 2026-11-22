import { useRef } from 'react';
import ArtFallback from '../../components/ArtFallback';
import FloatingPanel from '../../components/FloatingPanel';
import type { FloatingAnchor } from '../../components/FloatingPanel';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import { useDrawnPanel } from '../../dashboard/DrawnPanel';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { PanelFeed } from '../../dashboard/panelFeed';
import ResearchExplanation from '../research/ResearchExplanation';
import { TechnologyEffects } from '../research/TechnologyWindow';
import { researchAt } from '../research/researchData';
import { slotNumbers } from '../research/researchSlots';
import type { ResearchData, ResearchSlotsAt } from '../research/types';
import type { ActiveSlot } from './activeResearchData';
import { completedResearch } from './activeResearchData';
import { researchDate } from '../research/researchLayout';
import '../research/research.css';
import '../../components/Dossier.css';

export interface ResearchShown { token: string; name: string; icon: string | undefined }

export default function ResearchWindow({ runtime, widget, track, tag, data, reply, item, shown, feed, active, anchor, onClose }: {
  runtime: WidgetRuntime; widget: WidgetSpec; track: BoundTrack; tag: string; data: ResearchData; reply: ResearchSlotsAt; item: Extract<ActiveSlot, { kind: 'researching' }> | null;
  shown: ResearchShown; feed: PanelFeed | null; active: boolean; anchor: FloatingAnchor; onClose: () => void;
}) {
  const facts = useRef<HTMLDivElement>(null);
  useDrawnPanel(facts, runtime.clock, item ? feed : null, active);
  const numbers = item && slotNumbers(item.read, reply.hour);
  const slots = researchAt(data.history, reply.hour);
  const slot = item && (slots.find(owner => owner.index === item.slot.index) ?? item.slot);
  const finished = item ? null : completedResearch(slots, shown.token, reply.hour);
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
      {finished && <div className="dossier-progress is-done"><strong className="dossier-progress-value">Completed {researchDate(finished.hour)}</strong></div>}
      <div className="dossier-body">
      <TechnologyEffects runtime={runtime} widget={widget} tag={tag} token={shown.token} active={active} />
      {item && slot && (item.entry
        ? <section className="dossier-section"><h4>Research time</h4>
          <ResearchExplanation client={runtime.client} track={track} tag={tag} hour={reply.hour} history={data.history} slots={slots} slot={slot} entry={item.entry} daysLeft={numbers?.left ?? null} />
        </section>
        : <p className="dossier-missing">The research time breakdown is unavailable: this slot's history was not joined.</p>)}
      {finished && <section className="dossier-section"><h4>Research time</h4>
        <ResearchExplanation client={runtime.client} track={track} tag={tag} hour={reply.hour} history={data.history} slots={slots} slot={finished.slot} entry={finished.entry} />
      </section>}
      </div>
    </div>
  </FloatingPanel>;
}
