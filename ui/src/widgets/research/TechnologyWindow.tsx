import { useGate } from '../../dashboard/regionGate';
import { at } from '../../dashboard/regionTime';
import { useMemo, useState } from 'react';
import ArtFallback from '../../components/ArtFallback';
import FloatingPanel from '../../components/FloatingPanel';
import WidgetState from '../../components/WidgetState';
import { contentTitle, contentTitleHint } from '../../dashboard/contentTitles';
import { countryFromKey } from '../../dashboard/countryKey.ts';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useHeadProduct } from '../../dashboard/headProduct';
import type { WidgetBase } from '../../dashboard/model';
import EntityStatsWindow from './EntityStatsWindow';
import { GameArtFor, GameBlocks, type EntityLink } from './GameText';
import { technologyEffectsStructure, validateTechnologyEffects } from './technologyEffects';
import { useTechnologyArtwork } from './useTechnologyArtwork';
import '../../components/Dossier.css';
import './gameText.css';

interface Shown { runtime: WidgetRuntime; widget: WidgetBase; tag: string; hour: number; active: boolean }

export function LinkWindow({ link, onClose, ...shown }: Shown & { link: EntityLink; onClose: () => void }) {
  return link.kind === 'technology'
    ? <TechnologyWindow {...shown} link={link} onClose={onClose} />
    : <EntityStatsWindow {...shown} link={link} onClose={onClose} />;
}

export function OpenedLink({ opened, onClose, ...shown }: Shown & { opened: EntityLink | null; onClose: () => void }) {
  return opened && <LinkWindow key={JSON.stringify([opened.kind, opened.token, opened.databaseIndex, shown.hour])} {...shown} link={opened} onClose={onClose} />;
}

export function TechnologyEffects({ token, databaseIndex, ...shown }: Shown & { token: string; databaseIndex: number | null }) {
  const { runtime, widget, tag, hour, active } = shown;
  const action = useMemo(() => at(hour), [hour]);
  const params = useMemo(() => ({ country: countryFromKey(tag), token, databaseIndex }), [tag, token, databaseIndex]);
  const effects = useHeadProduct(runtime, useGate(runtime, widget, action, active), 'technology-effects', params,
    { active, validate: validateTechnologyEffects, structure: technologyEffectsStructure });
  const [opened, setOpened] = useState<EntityLink | null>(null);
  const { data, error, retry } = effects;
  const refusals = Object.entries(data?.refusals ?? {});
  return <GameArtFor runtime={runtime} widget={widget} active={active}><section className="dossier-section" ref={effects.root}>
    <h4>Effects</h4>
    {error ? <WidgetState kind="error" what="technology effects" failure={error} onRetry={retry} />
      : !data ? <p className="dossier-missing">Loading effects…</p>
        : <>
          {data.unavailable && <p className="dossier-missing">{data.unavailable}</p>}
          {data.runs.length ? <GameBlocks runs={data.runs} onOpen={setOpened} /> : !data.unavailable && !refusals.length && <p className="dossier-missing">No effects to describe.</p>}
          {refusals.map(([reason, count]) => <p className="dossier-missing" key={reason}>Not recorded: {reason}{(count ?? 0) > 1 ? ` (${count})` : ''}</p>)}
        </>}
    <OpenedLink {...shown} opened={opened} onClose={() => setOpened(null)} />
  </section></GameArtFor>;
}

export default function TechnologyWindow({ link, onClose, ...shown }: Shown & { link: EntityLink; onClose: () => void }) {
  const { runtime, widget, active } = shown;
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const tokens = useMemo(() => [link.token], [link.token]);
  const recorded = useMemo(() => link.databaseIndex === null ? undefined : new Map([[link.token, [link.databaseIndex]]]), [link.token, link.databaseIndex]);
  const read = useTechnologyArtwork(runtime.client, track, tokens, active, recorded);
  const art = read.artwork.get(link.token), failed = read.failures.get(link.token);
  const name = contentTitle(failed ?? art?.name, link.token, 'Technology name unavailable', !art && !failed);
  return <FloatingPanel title="Technology" label={`${name} technology`} anchor={{ x: link.x, y: link.y }} onClose={onClose} className="technology-window">
    <div className="dossier">
      <header className="dossier-hero">
        <span className="dossier-art">{art?.icon ? <img src={art.icon} alt="" /> : <ArtFallback icon="flask" />}</span>
        <div className="dossier-title">
          <h3 data-tooltip={contentTitleHint(link.token)}>{name}</h3>
        </div>
      </header>
      <div className="dossier-body">
        <TechnologyEffects {...shown} token={link.token} databaseIndex={link.databaseIndex} />
      </div>
    </div>
  </FloatingPanel>;
}
