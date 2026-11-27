import { useRef, useState, type RefObject } from 'react';
import { jumpItem } from '../../charts/jump.ts';
import { BattleViewer } from '../../components/BattleViewer.tsx';
import { formatDay } from '../../dashboard/calendar';
import type { FamilyViewProps, FamilyViewRenderer, WidgetRuntime } from '../../dashboard/definitions';
import type { BoundTrack } from '../../dashboard/model';
import { useProduct } from '../../dashboard/products.ts';
import { classifyFailure, failureText } from '../../dashboard/queryError.ts';
import type { GatedHour } from '../../dashboard/regionTime.ts';
import { productRequest } from '../../dashboard/request.ts';
import { mirrorRowSelector } from '../../families/host/mirror.ts';
import { FamilyWidget } from '../../families/host/registry.tsx';
import { validateNavalBattleCourse } from '../naval-battles/navalBattleCourse.ts';
import NavalBattleWindow from '../naval-battles/NavalBattleWindow.tsx';
import type { SinkingSubject } from './shipsSunkProjection.ts';

interface Asked {
  readonly scope: string;
  readonly row: string;
  readonly battle: NonNullable<SinkingSubject['battle']>;
  readonly sunk: number;
  readonly hour: number;
}

function SunkBattle({ asked, track, runtime, anchor, onClose }: {
  readonly asked: Asked;
  readonly track: BoundTrack;
  readonly runtime: WidgetRuntime;
  readonly anchor: RefObject<HTMLDivElement | null>;
  readonly onClose: () => void;
}) {
  const course = useProduct(runtime.client, productRequest({ sourceId: track.source.id, revision: track.source.revision,
    product: 'naval-battle-course', params: { branchId: track.branch.id, key: asked.battle.key, hour: asked.hour as GatedHour } }), true, validateNavalBattleCourse);
  const row = course.data?.row;
  if (row) {
    return <NavalBattleWindow opened={{ scope: asked.scope, subject: { row, title: asked.battle.title }, hour: asked.hour, start: asked.sunk }}
      track={track} runtime={runtime} anchor={anchor} returnFocus={mirrorRowSelector(asked.row)} onClose={onClose} />;
  }
  const reason = course.error ? failureText(course.error, 'The battle couldn’t be read.')
    : course.data ? 'The recording holds no reading of this battle by the dashboard’s hour.' : null;
  const unplayable = reason ? <>
    <span className="bv-missing" data-tooltip={reason} tabIndex={0}><i className="bv-hatch" aria-hidden="true" />Battle not available</span>
    {course.error && classifyFailure(course.error)?.kind === 'retryable' && <button type="button" className="button small" onClick={course.retry}>Try again</button>}
  </> : <span className="lb-note" aria-busy="true">Reading the battle…</span>;
  return <BattleViewer subject={asked.battle.key} title={asked.battle.title} day={<span className="bv-when"><time>{formatDay(asked.sunk)}</time></span>}
    closeLabel="Close battle viewer" anchor={anchor} returnFocus={mirrorRowSelector(asked.row)} runtime={runtime} track={track} province={null}
    span={null} unplayable={unplayable} gaps={[]} fallbackHour={asked.sunk} onClose={onClose} views={['battle']}
    sides={() => null} battle={() => null} />;
}

function ShipsSunkView(props: FamilyViewProps) {
  const anchor = useRef<HTMLDivElement>(null);
  const track = props.runtime.tracks.find(item => item.id === props.widget.trackIds[0]);
  const scope = track && JSON.stringify([track.source.id, track.source.revision, track.branch.id]);
  const [asked, setAsked] = useState<Asked | null>(null);
  const shown = asked && asked.scope === scope ? asked : null;
  return props.manifestDeclaration ? <div ref={anchor} className="fh-detail-anchor">
    <FamilyWidget {...props} family="table" declaration={props.manifestDeclaration} selectedKey={shown?.row}
      onActivate={action => {
        const subject = action.meta as SinkingSubject | undefined;
        if (scope && subject?.battle && action.target.kind === 'row') setAsked({ scope, row: action.target.key, battle: subject.battle, sunk: subject.hour, hour: action.hour });
      }}
      contextMenuItems={hit => track && hit.meta ? [jumpItem(props.runtime, track.id, (hit.meta as SinkingSubject).hour)] : []}
    />
    {shown && track && <SunkBattle key={shown.row} asked={shown} track={track} runtime={props.runtime} anchor={anchor} onClose={() => setAsked(null)} />}
  </div> : null;
}

export default { render: ShipsSunkView } satisfies FamilyViewRenderer;
