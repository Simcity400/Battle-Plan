import { createContext, useContext, useMemo } from 'react';
import type { MouseEvent, ReactNode } from 'react';
import type { WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack, WidgetSpec } from '../../dashboard/model';
import { useSpriteArtwork } from '../../dashboard/useSpriteArtwork';
import { gameBlocks } from './gameLayout';
import type { GameBlock } from './gameLayout';
import type { EntityKind, GameRun } from '../../generated/dashboard';
import { ENTITY_LABELS, toneClass } from './gameRuns';
import './gameText.css';

export interface GameArtSource { runtime: WidgetRuntime; track: BoundTrack | undefined; active: boolean }

export const GameArt = createContext<GameArtSource | null>(null);
const Icons = createContext<((sprite: string) => string | null) | null>(null);

function DrawnIcons({ art, runs, children }: { art: GameArtSource; runs: readonly GameRun[]; children: ReactNode }) {
  const sprites = useMemo(() => runs.flatMap(run => run.kind === 'icon' ? [{ key: run.sprite, frame: 0, text: true }] : []), [runs]);
  const icon = useSpriteArtwork(art.runtime, art.track, sprites, art.active);
  return <Icons.Provider value={sprite => icon({ key: sprite, frame: 0, text: true })}>{children}</Icons.Provider>;
}

export function GameArtFor({ runtime, widget, active, children }: { runtime: WidgetRuntime; widget: WidgetSpec; active: boolean; children: ReactNode }) {
  const track = runtime.tracks.find(item => item.id === widget.trackIds[0]);
  const art = useMemo(() => ({ runtime, track, active }), [runtime, track, active]);
  return <GameArt.Provider value={art}>{children}</GameArt.Provider>;
}

export function GameIcons({ runs, children }: { runs: readonly GameRun[]; children: ReactNode }) {
  const art = useContext(GameArt);
  return art ? <DrawnIcons art={art} runs={runs}>{children}</DrawnIcons> : <>{children}</>;
}

function GameIcon({ sprite }: { sprite: string }) {
  const icon = useContext(Icons)?.(sprite) ?? null;
  return icon
    ? <img className="game-text-icon" src={icon} alt={sprite} title={`${sprite} · art from the current game installation`} />
    : <span className="game-text-icon is-missing" role="img" aria-label={`Icon ${sprite} not available`} title={`Icon ${sprite} not available`}>{sprite}</span>;
}

export interface EntityLink { kind: EntityKind; token: string; databaseIndex: number | null; text: string; x: number; y: number }

const toned = (base: string, tone: string | null) => {
  const shade = toneClass(tone);
  return shade ? `${base} is-${shade}` : base;
};

export function GameRuns({ runs, onOpen }: { runs: readonly GameRun[]; onOpen?: (link: EntityLink) => void }) {
  const drawn = useContext(Icons);
  const body = <>{runs.map((run, at) => {
    switch (run.kind) {
      case 'text': return toneClass(run.tone) ? <span key={at} className={toned('game-text-run', run.tone)}>{run.text}</span> : <span key={at}>{run.text}</span>;
      case 'icon': return <GameIcon key={at} sprite={run.sprite} />;
      case 'flag': return <span key={at} className="game-text-flag" title={`Flag of ${run.tag}`}>{run.tag}</span>;
      case 'link': {
        const open = onOpen && ((event: MouseEvent<HTMLButtonElement>) => onOpen({ kind: run.entity, token: run.token, databaseIndex: run.databaseIndex, text: run.text, x: event.clientX, y: event.clientY }));
        return open
          ? <button key={at} type="button" className={toned('game-text-link', run.tone)} onClick={open}
            data-tooltip={run.entity === 'technology' ? 'Technology effects' : `${ENTITY_LABELS[run.entity]} stats`}>{run.text}</button>
          : <span key={at} className={toned('game-text-run', run.tone)}>{run.text}</span>;
      }
    }
  })}</>;
  return drawn ? body : <GameIcons runs={runs}>{body}</GameIcons>;
}

export default function GameText({ runs, onOpen, className = '' }: { runs: readonly GameRun[]; onOpen?: (link: EntityLink) => void; className?: string }) {
  return <GameIcons runs={runs}><p className={`game-text ${className}`}><GameRuns runs={runs} onOpen={onOpen} /></p></GameIcons>;
}

export function GameBlocks({ runs, onOpen }: { runs: readonly GameRun[]; onOpen?: (link: EntityLink) => void }) {
  const blocks = gameBlocks(runs);
  const out: ReactNode[] = [];
  for (let at = 0; at < blocks.length; at++) {
    const block = blocks[at];
    if (block.kind === 'stat') {
      const rows = [];
      for (; at < blocks.length && blocks[at].kind === 'stat'; at++) {
        const row = blocks[at] as Extract<GameBlock, { kind: 'stat' }>;
        rows.push(<div key={at}><dt><GameRuns runs={row.label} onOpen={onOpen} /></dt><dd><GameRuns runs={row.value} onOpen={onOpen} /></dd></div>);
      }
      at--;
      out.push(<dl key={'s' + at} className="game-block-stats">{rows}</dl>);
    } else if (block.kind === 'heading') out.push(<p key={at} className="game-block-heading"><GameRuns runs={block.runs} onOpen={onOpen} /></p>);
    else out.push(<p key={at} className={block.quiet ? 'game-block-text is-quiet' : 'game-block-text'}><GameRuns runs={block.runs} onOpen={onOpen} /></p>);
  }
  return <GameIcons runs={runs}><div className="game-blocks">{out}</div></GameIcons>;
}
