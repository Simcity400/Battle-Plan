import { useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import ArtFallback from '../../components/ArtFallback';
import FloatingPanel from '../../components/FloatingPanel';
import type { FloatingAnchor } from '../../components/FloatingPanel';
import ProgressMeter from '../../charts/meter/ProgressMeter';
import { useDrawnPanel } from '../../dashboard/DrawnPanel';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import { useHeadProduct } from '../../dashboard/headProduct';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import type { PanelFeed } from '../../dashboard/panelFeed';
import ResearchExplanation from '../research/ResearchExplanation';
import { researchAt } from '../research/researchData';
import { slotNumbers } from '../research/researchSlots';
import { technologyEffectsStructure, validateTechnologyEffects } from '../research/technologyEffects';
import type { TechnologyEffects } from '../../generated/dashboard';
import type { ResearchData, ResearchSlotsAt } from '../research/types';
import type { ActiveSlot } from './activeResearchData';
import '../research/research.css';

export interface ResearchShown { token: string; name: string; icon: string | undefined; description: string | null; folder: string | null }

export default function ResearchWindow({ runtime, widget, track, tag, data, reply, item, shown, feed, active, anchor, onClose }: {
  runtime: WidgetRuntime; widget: WidgetSpec; track: BoundTrack; tag: string; data: ResearchData; reply: ResearchSlotsAt; item: Extract<ActiveSlot, { kind: 'researching' }> | null;
  shown: ResearchShown; feed: PanelFeed | null; active: boolean; anchor: FloatingAnchor; onClose: () => void;
}) {
  const facts = useRef<HTMLDListElement>(null);
  useDrawnPanel(facts, runtime.clock, item ? feed : null, active);
  const numbers = item && slotNumbers(item.read, reply.hour);
  const slots = researchAt(data.history, reply.hour);
  const slot = item && (slots.find(owner => owner.index === item.slot.index) ?? item.slot);
  return <FloatingPanel title="Research" label="research details" anchor={anchor} onClose={onClose} className="research-window">
    <div className="research-window-page">
      <header className="research-window-head">
        <span className="research-window-art">{shown.icon ? <img src={shown.icon} alt="" /> : <ArtFallback icon="flask" />}</span>
        <div className="research-window-title">
          <h3>{shown.name}</h3>
          <p>{shown.folder ?? 'Folder not recorded'}{item && <> · Slot {item.slot.index + 1}</>}</p>
        </div>
        <span className={`research-window-state${item ? '' : ' is-idle'}`}>{item ? 'Researching' : 'Not researching'}</span>
      </header>
      {item && numbers ? <dl className="research-window-facts" ref={facts} data-research-slot={item.slot.index}>
        <div className="research-window-progress"><dt>Progress</dt>
          <dd><ProgressMeter value={numbers.fraction} state="live" label={`${shown.name} progress`} /><strong data-paint="percent">{numbers.percent}</strong></dd></div>
        <div><dt>Time left</dt><dd><span data-paint="days">{numbers.days}</span></dd></div>
      </dl> : <p className="research-window-missing">This technology is not being researched at this time.</p>}
      {shown.description ? <p className="research-window-description">{shown.description}</p> : <p className="research-window-missing">No description recorded.</p>}
      <Effects runtime={runtime} widget={widget} tag={tag} token={shown.token} active={active} />
      {item && slot && (item.entry
        ? <section className="research-window-section"><h4>Research time</h4>
          <ResearchExplanation client={runtime.client} track={track} tag={tag} hour={reply.hour} history={data.history} slots={slots} slot={slot} entry={item.entry} />
        </section>
        : <p className="research-window-missing">The research time breakdown is unavailable: this slot's history was not joined.</p>)}
    </div>
  </FloatingPanel>;
}

function Effects({ runtime, widget, tag, token, active }: { runtime: WidgetRuntime; widget: WidgetSpec; tag: string; token: string; active: boolean }) {
  const effects = useHeadProduct<TechnologyEffects>(runtime, widget, 'technology-effects', { country: countryFromKey(tag), token },
    { active, validate: validateTechnologyEffects, structure: technologyEffectsStructure });
  const { data, error, retry } = effects;
  const refusals = Object.entries(data?.refusals ?? {});
  return <section className="research-window-section" ref={effects.root}>
    <h4>Effects</h4>
    {error ? <WidgetState kind="error" failure={error} onRetry={retry} />
      : !data ? <p className="research-window-missing">Loading effects…</p>
        : <>
          {data.unavailable && <p className="research-window-missing">{data.unavailable}</p>}
          {data.text ? <p className="research-window-effects">{data.text}</p> : !data.unavailable && !refusals.length && <p className="research-window-missing">No effects to describe.</p>}
          {refusals.map(([reason, count]) => <p className="research-window-missing" key={reason}>{reason} ({count})</p>)}
        </>}
  </section>;
}
