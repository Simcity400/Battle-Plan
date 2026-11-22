import type { MouseEvent, ReactNode } from 'react';
import { gameBlocks } from './gameLayout';
import type { GameBlock } from './gameLayout';
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
  return <div className="game-blocks">{out}</div>;
}
