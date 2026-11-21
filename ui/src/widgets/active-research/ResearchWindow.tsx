import { useRef, useState } from 'react';
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
import GameText, { type EntityLink } from '../research/GameText';
import EntityStatsWindow from '../research/EntityStatsWindow';
import type { TechnologyEffects } from '../../generated/dashboard';
import type { ResearchData, ResearchSlotsAt } from '../research/types';
import type { ActiveSlot } from './activeResearchData';
import '../research/research.css';
import '../../components/Dossier.css';

export interface ResearchShown { token: string; name: string; icon: string | undefined; description: string | null; folder: string | null }

export default function ResearchWindow({ runtime, widget, track, tag, data, reply, item, shown, feed, active, anchor, onClose }: {
  runtime: WidgetRuntime; widget: WidgetSpec; track: BoundTrack; tag: string; data: ResearchData; reply: ResearchSlotsAt; item: Extract<ActiveSlot, { kind: 'researching' }> | null;
  shown: ResearchShown; feed: PanelFeed | null; active: boolean; anchor: FloatingAnchor; onClose: () => void;
}) {
  const facts = useRef<HTMLDivElement>(null);
  useDrawnPanel(facts, runtime.clock, item ? feed : null, active);
  const numbers = item && slotNumbers(item.read, reply.hour);
  const slots = researchAt(data.history, reply.hour);
  const slot = item && (slots.find(owner => owner.index === item.slot.index) ?? item.slot);
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
      <div className="dossier-body">
      {!item && <p className="dossier-missing">This technology is not being researched at this time.</p>}
      {shown.description ? <p className="dossier-lede">{shown.description}</p> : <p className="dossier-missing">No description recorded.</p>}
      <Effects runtime={runtime} widget={widget} tag={tag} token={shown.token} active={active} />
      {item && slot && (item.entry
        ? <section className="dossier-section"><h4>Research time</h4>
          <ResearchExplanation client={runtime.client} track={track} tag={tag} hour={reply.hour} history={data.history} slots={slots} slot={slot} entry={item.entry} daysLeft={numbers?.left ?? null} />
        </section>
        : <p className="dossier-missing">The research time breakdown is unavailable: this slot's history was not joined.</p>)}
      </div>
    </div>
  </FloatingPanel>;
}

function Effects({ runtime, widget, tag, token, active }: { runtime: WidgetRuntime; widget: WidgetSpec; tag: string; token: string; active: boolean }) {
  const effects = useHeadProduct<TechnologyEffects>(runtime, widget, 'technology-effects', { country: countryFromKey(tag), token },
    { active, validate: validateTechnologyEffects, structure: technologyEffectsStructure });
  const [opened, setOpened] = useState<EntityLink | null>(null);
  const { data, error, retry } = effects;
  const refusals = Object.entries(data?.refusals ?? {});
  return <section className="dossier-section" ref={effects.root}>
    <h4>Effects</h4>
    {error ? <WidgetState kind="error" failure={error} onRetry={retry} />
      : !data ? <p className="dossier-missing">Loading effects…</p>
        : <>
          {data.unavailable && <p className="dossier-missing">{data.unavailable}</p>}
          {data.runs.length ? <GameText runs={data.runs} onOpen={setOpened} /> : !data.unavailable && !refusals.length && <p className="dossier-missing">No effects to describe.</p>}
          {refusals.map(([reason, count]) => <p className="dossier-missing" key={reason}>Not recorded: {reason}{(count ?? 0) > 1 ? ` (${count})` : ''}</p>)}
        </>}
    {opened && <EntityStatsWindow key={`${opened.kind}:${opened.token}`} runtime={runtime} widget={widget} tag={tag} link={opened} active={active} onClose={() => setOpened(null)} />}
  </section>;
}
