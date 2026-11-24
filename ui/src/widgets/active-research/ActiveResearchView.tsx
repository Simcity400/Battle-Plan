import { useCallback, useMemo, useRef, useState } from 'react';
import ArtFallback from '../../components/ArtFallback';
import type { FamilyViewProps } from '../../dashboard/definitions';
import type { WidgetBase } from '../../dashboard/model';
import { openedOn } from '../../dashboard/gameWindows';
import { useGate } from '../../dashboard/regionGate';
import type { InstalledAction } from '../../families/contract.ts';
import { DetailWindowHost } from '../../families/host/index.ts';
import { useArtResolutions } from '../../families/art/react.ts';
import type { ArtHandle } from '../../families/plan.ts';
import { researchAt } from '../research/researchData';
import ResearchExplanation from '../research/ResearchExplanation';
import { TechnologyEffects } from '../research/TechnologyWindow';
import { useResearchReply } from '../research/useResearch';
import type { ActiveResearchRowMeta } from './activeResearchProjection.ts';
import '../research/research.css';
import '../../components/Dossier.css';
import { FamilyWidget } from '../../families/host/registry.tsx';

const NO_HANDLES: readonly ArtHandle[] = [];

export default function ActiveResearchView(props: FamilyViewProps) {
  const { widget, runtime, active } = props;
  const hostRef = useRef<HTMLDivElement>(null);
  const history = useResearchReply(widget, runtime, active);
  const { track, tag, data } = history;
  const [opened, setOpened] = useState<{
    readonly on: string;
    readonly key: string;
    readonly meta: ActiveResearchRowMeta;
    readonly hour: number;
  } | null>(null);

  const onActivate = useCallback((action: InstalledAction) => {
    if (!track || !tag || !action.meta || typeof action.meta !== 'object') return;
    const meta = action.meta as ActiveResearchRowMeta;
    if (!meta.token) return;
    setOpened({ on: openedOn(track, tag), key: action.subject, meta, hour: action.hour });
  }, [track, tag]);

  const shown = track && tag && opened?.on === openedOn(track, tag) ? opened : null;
  const detailWidget = useMemo<WidgetBase>(() => {
    if (!shown || !track) return widget;
    const offset = shown.hour - track.branch.firstHour;
    return { ...widget, time: { mode: 'pin', windows: { [track.id]: { firstOffset: offset, lastOffset: offset } } } };
  }, [shown, track, widget]);
  const gate = useGate(runtime, widget);
  const detailSlots = useMemo(
    () => (shown && data ? researchAt(data.history, shown.hour) : []),
    [shown, data],
  );
  const detailSlot = shown ? detailSlots.find(s => s.index === shown.meta.slot) ?? null : null;
  const detailEntry = shown && detailSlot && shown.meta.opened.firstObservedHour !== null
    ? detailSlot.entries.find(e => e.token === shown.meta.token && e.databaseIndex === shown.meta.databaseIndex && e.firstObservedHour === shown.meta.opened.firstObservedHour) ?? null
    : null;
  const detailHandles = useMemo<readonly ArtHandle[]>(
    () => (shown ? [{ kind: 'technology', token: shown.meta.token, databaseIndex: shown.meta.databaseIndex }] : NO_HANDLES),
    [shown],
  );
  const resolveArt = useArtResolutions(runtime, track, detailHandles, active && detailHandles.length > 0);
  const detailArt = detailHandles[0] ? resolveArt(detailHandles[0]) : null;
  const detailIcon = detailArt?.status === 'ready' ? detailArt.url : undefined;

  return <div ref={hostRef} className="fh-detail-anchor">
    {props.manifestDeclaration ? <FamilyWidget
      {...props}
      family={props.manifestDeclaration.family} declaration={props.manifestDeclaration}
      selectedKey={shown?.key ?? null}
      onActivate={onActivate}
    /> : null}
    <DetailWindowHost
      open={Boolean(shown && track && tag)}
      subject={shown ? `${tag}:${shown.meta.token}` : null}
      hour={shown?.hour ?? null}
      title={shown?.meta.name ?? 'Research'}
      anchor={hostRef}
      onClose={() => setOpened(null)}
    >
      {shown && track && tag && (
        <div className="dossier">
          <header className="dossier-hero">
            <span className="dossier-art">{detailIcon ? <img src={detailIcon} alt="" /> : <ArtFallback icon="flask" />}</span>
            <div className="dossier-title">
              <h3>{shown.meta.name}</h3>
            </div>
          </header>
          <div className="dossier-body">
            <TechnologyEffects
              runtime={runtime}
              widget={detailWidget}
              tag={tag}
              token={shown.meta.token}
              databaseIndex={shown.meta.databaseIndex}
              active={active}
            />
            {data && detailSlot && (detailEntry
              ? <section className="dossier-section">
                  <h4>Research time</h4>
                  <ResearchExplanation
                    client={runtime.client}
                    gate={gate}
                    track={track}
                    tag={tag}
                    hour={shown.hour}
                    history={data.history}
                    slots={detailSlots}
                    slot={detailSlot}
                    entry={detailEntry}
                  />
                </section>
              : <p className="dossier-missing">The research time breakdown is unavailable: this slot’s history was not joined.</p>)}
          </div>
        </div>
      )}
    </DetailWindowHost>
  </div>;
}
