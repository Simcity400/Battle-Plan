import type { MouseEvent } from 'react';
import type { EntityKind, GameRun } from '../../generated/dashboard';
import { ENTITY_LABELS, toneClass } from './gameRuns';
import './gameText.css';

export interface EntityLink { kind: EntityKind; token: string; text: string; x: number; y: number }

const toned = (base: string, tone: string | null) => {
  const shade = toneClass(tone);
  return shade ? `${base} is-${shade}` : base;
};

export function GameRuns({ runs, onOpen }: { runs: readonly GameRun[]; onOpen?: (link: EntityLink) => void }) {
  return <>{runs.map((run, at) => {
    switch (run.kind) {
      case 'text': return toneClass(run.tone) ? <span key={at} className={toned('game-text-run', run.tone)}>{run.text}</span> : <span key={at}>{run.text}</span>;
      case 'icon': return <span key={at} className="game-text-icon" title={run.sprite} aria-label={`Icon ${run.sprite}`} role="img" />;
      case 'flag': return <span key={at} className="game-text-flag" title={`Flag of ${run.tag}`}>{run.tag}</span>;
      case 'link': {
        const open = onOpen && ((event: MouseEvent<HTMLButtonElement>) => onOpen({ kind: run.entity, token: run.token, text: run.text, x: event.clientX, y: event.clientY }));
        return open
          ? <button key={at} type="button" className={toned('game-text-link', run.tone)} onClick={open}
            data-tooltip={run.entity === 'technology' ? 'Technology effects' : `${ENTITY_LABELS[run.entity]} stats`}>{run.text}</button>
          : <span key={at} className={toned('game-text-run', run.tone)}>{run.text}</span>;
      }
    }
  })}</>;
}

export default function GameText({ runs, onOpen, className = '' }: { runs: readonly GameRun[]; onOpen?: (link: EntityLink) => void; className?: string }) {
  return <p className={`game-text ${className}`}><GameRuns runs={runs} onOpen={onOpen} /></p>;
}
