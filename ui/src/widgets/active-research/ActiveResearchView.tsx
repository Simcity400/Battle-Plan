import { useCallback, useMemo, useRef, useState } from 'react';
import ArtFallback from '../../components/ArtFallback';
import type { WidgetProps } from '../../dashboard/definitions';
import type { WidgetSpec } from '../../dashboard/model';
import { openedOn } from '../../dashboard/gameWindows';
import { useGate } from '../../dashboard/regionGate';
import { artHandleKey, failedArt, missingArt, readyArt, type ArtResolution } from '../../families/art/types.ts';
import type { InstalledAction } from '../../families/contract.ts';
import { DetailWindowHost } from '../../families/host/index.ts';
import DeclaredFamilyWidget from '../../families/host/DeclaredFamilyWidget.tsx';
import { meterHostAdapter } from '../../families/meter/hostAdapter.ts';
import { researchAt, researchArtwork, researchRecordedTemplates } from '../research/researchData';
import ResearchExplanation from '../research/ResearchExplanation';
import { TechnologyEffects } from '../research/TechnologyWindow';
import { historyTokens, useResearchReply } from '../research/useResearch';
import { useTechnologyArtwork } from '../research/useTechnologyArtwork';
import { activeResearchDeclaration, type ActiveResearchRowMeta } from './activeResearchProjection.ts';
import '../research/research.css';
import '../../components/Dossier.css';

const NO_TOKENS: readonly string[] = [];

export default function ActiveResearchView(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const hostRef = useRef<HTMLDivElement>(null);
  const history = useResearchReply(widget, runtime, active);
  const { track, tag, data } = history;
  const tokens = useMemo(
    () => (data ? [...new Set(historyTokens(data.history).map(use => (typeof use === 'string' ? use : use[0])))] : NO_TOKENS),
    [data],
  );
  const recorded = useMemo(() => researchRecordedTemplates(data?.history), [data]);
  const images = useTechnologyArtwork(runtime.client, track, tokens, active && !!data, recorded);
  const artwork = useMemo(() => researchArtwork(data, images.artwork), [data, images.artwork]);

  const artResolutions = useMemo<Readonly<Record<string, ArtResolution>>>(() => {
    const out: Record<string, ArtResolution> = {};
    for (const token of tokens) {
      const fail = images.failures.get(token);
      if (fail) {
        out[artHandleKey({ kind: 'sprite', key: `technology:${token}`, frame: 0 })] = failedArt(fail.message, true);
        continue;
      }
      const entry = images.artwork.get(token);
      if (entry) {
        out[artHandleKey({ kind: 'sprite', key: `technology:${token}`, frame: 0 })] = entry.icon
          ? readyArt(entry.icon, { assetId: `technology:${token}` })
          : missingArt('Art missing from the current installation');
      }
    }
    return out;
  }, [tokens, images.artwork, images.failures]);

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
  const detailWidget = useMemo<WidgetSpec>(() => {
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
  const detailIcon = shown ? artwork.get(shown.meta.token)?.icon ?? undefined : undefined;

  return <div ref={hostRef} className="fh-detail-anchor">
    <DeclaredFamilyWidget
      {...props}
      adapter={meterHostAdapter}
      declaration={activeResearchDeclaration}
      artResolutions={artResolutions}
      selectedKey={shown?.key ?? null}
      onActivate={onActivate}
      extraWarnings={images.warnings}
    />
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
