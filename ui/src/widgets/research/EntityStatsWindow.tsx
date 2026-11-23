import { useGate } from '../../dashboard/regionGate';
import WidgetState from '../../components/WidgetState';
import FloatingPanel from '../../components/FloatingPanel';
import { contentTitleHint } from '../../dashboard/contentTitles';
import type { WidgetRuntime } from '../../dashboard/definitions';
import { useHeadProduct } from '../../dashboard/headProduct';
import type { WidgetSpec } from '../../dashboard/model';
import { useMemo } from 'react';
import { GameArtFor, GameIcons, GameRuns } from './GameText';
import type { EntityLink } from './GameText';
import { entityStatsStructure, validateEntityStats } from './entityStats';
import { ENTITY_LABELS, trimLabel, trimValue } from './gameRuns';
import './gameText.css';

export default function EntityStatsWindow({ runtime, widget, link, active, onClose }: {
  runtime: WidgetRuntime; widget: WidgetSpec; tag: string; link: EntityLink; active: boolean; onClose: () => void;
}) {
  const stats = useHeadProduct(runtime, useGate(runtime, widget), 'content-entity-stats', { kind: link.kind, token: link.token },
    { active, validate: validateEntityStats, structure: entityStatsStructure });
  const { data, error, retry } = stats;
  const kind = ENTITY_LABELS[link.kind];
  const refusals = Object.entries(data?.refusals ?? {});
  const name = link.text.trim() || link.token;
  const runs = useMemo(() => data?.stats.flatMap(stat => [...stat.label, ...stat.value]) ?? [], [data]);
  return <FloatingPanel title={kind} label={`${kind} ${name} stats`} anchor={{ x: link.x, y: link.y }} onClose={onClose} className="entity-stats-window">
    <GameArtFor runtime={runtime} widget={widget} active={active}><GameIcons runs={runs}><div className="entity-stats" ref={stats.root}>
      <h3 data-tooltip={contentTitleHint(link.token)}>{name}</h3>
      {error ? <WidgetState kind="error" failure={error} onRetry={retry} />
        : !data ? <p className="entity-stats-missing">Loading stats…</p>
          : <>
            {data.unavailable && <p className="entity-stats-missing">{data.unavailable}</p>}
            {data.stats.length > 0 && <dl className="entity-stats-list">
              {data.stats.map((stat, at) => <div key={at}>
                <dt><GameRuns runs={trimLabel(stat.label)} />{stat.source && <small className="entity-stats-source">{stat.source}</small>}</dt>
                <dd><GameRuns runs={trimValue(stat.value)} /></dd>
              </div>)}
            </dl>}
            {!data.unavailable && !data.stats.length && !refusals.length && <p className="entity-stats-missing">No stats recorded.</p>}
            {refusals.map(([reason, count]) => <p className="entity-stats-missing" key={reason}>Withheld: {reason}{(count ?? 0) > 1 ? ` (${count})` : ''}</p>)}
          </>}
    </div></GameIcons></GameArtFor>
  </FloatingPanel>;
}
