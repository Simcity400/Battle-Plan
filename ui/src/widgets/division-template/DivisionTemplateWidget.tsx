import { useCallback, useEffect, useId, useRef, useState, useSyncExternalStore } from 'react';
import WidgetState from '../../components/WidgetState';
import FollowControl from '../../charts/drill/FollowControl';
import { useDrill } from '../../charts/drill/useDrill';
import GateState from '../../dashboard/GateState';
import { DetailWindowHost } from '../../families/host/DetailWindowHost';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { TimeRegion, useRegion, useRegionCoverage, useRegionHour } from '../../dashboard/widgetTime.ts';
import type { InstalledAction } from '../../families/contract.ts';
import { FamilyWidget } from '../../families/host/registry';
import {
  claimGameWindowSlot,
  gameWindow,
  releaseGameWindowSlot,
  subscribeGameWindows,
  updateGameWindowSlotPosition,
} from '../../dashboard/gameWindows';
import type { WindowPosition } from '../../dashboard/gameWindows';
import { divisionTemplateDeclaration } from './divisionTemplateProjection';
import './divisionTemplate.css';

const NO_LINES: readonly string[] = [];
const divisionDesignerWindow = () => gameWindow('divisionDesigner');

export default function DivisionTemplateWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const drillRegion = useRegion(widget, drillControl.region.action);
  const drillCoverage = useRegionCoverage(runtime, drillRegion);
  const drillAt = useRegionHour(runtime, drillRegion, active && drill.breakdown !== null);
  const Designer = useSyncExternalStore(subscribeGameWindows, divisionDesignerWindow, divisionDesignerWindow);
  const anchor = useRef<HTMLDivElement>(null);
  const [opened, setOpened] = useState<{ template: string; designer: 'ordinary' | 'headquarters'; tag: string } | null>(null);
  const slotOwnerId = useId();
  const [inheritedPos, setInheritedPos] = useState<WindowPosition | null>(null);
  const [windowLines, setWindowLines] = useState<readonly string[]>(NO_LINES);
  const onWindowDiagnostics = useCallback((lines: string[]) =>
    setWindowLines(prev => (prev.length === lines.length && prev.every((line, i) => line === lines[i]) ? prev : lines)), []);

  const closeForHandover = useCallback(() => {
    dispatch({ type: 'close' });
    setOpened(null);
    setWindowLines(NO_LINES);
    setInheritedPos(null);
  }, [dispatch]);
  const closeDrill = useCallback(() => {
    releaseGameWindowSlot('divisionDesigner', slotOwnerId);
    closeForHandover();
  }, [slotOwnerId, closeForHandover]);

  useEffect(() => closeDrill(), [track?.source.id, track?.source.revision, track?.branch.id, widget.options?.country, closeDrill]);
  useEffect(() => () => releaseGameWindowSlot('divisionDesigner', slotOwnerId), [slotOwnerId]);

  const handleActivate = useCallback((action: InstalledAction) => {
    if (!Designer) return;
    const info = action.meta as { designer?: 'ordinary' | 'headquarters' | null; tag?: string } | null | undefined;
    if (!info?.tag || !info.designer) return;
    setInheritedPos(claimGameWindowSlot('divisionDesigner', slotOwnerId, closeForHandover));
    setOpened({ template: action.subject, designer: info.designer, tag: info.tag });
    drillControl.show(action.subject, action.hour);
  }, [Designer, drillControl, slotOwnerId, closeForHandover]);
  const onPositionChange = useCallback((pos: WindowPosition) => {
    updateGameWindowSlotPosition('divisionDesigner', slotOwnerId, pos);
  }, [slotOwnerId]);

  const main = <FamilyWidget
    {...props}
    family="table"
    declaration={divisionTemplateDeclaration}
    selectedKey={opened?.template ?? null}
    onActivate={Designer ? handleActivate : undefined}
    extraWarnings={windowLines}
  />;

  const shut = drillControl.gate.stage.stage !== 'open' && drillControl.gate.stage.stage !== 'unbound';

  return <div ref={anchor} className="division-template">
    {main}
    <DetailWindowHost
      open={drill.breakdown !== null && !!Designer && !!opened}
      subject={opened?.template ?? null}
      hour={drillAt}
      title="Division designer"
      day={shut || drillAt === null ? '' : formatDateTime(drillAt)}
      placement="center"
      anchor={anchor}
      initialPosition={inheritedPos}
      onPositionChange={onPositionChange}
      headerActions={<FollowControl following={drillControl.following} onFollow={drillControl.follow} onPause={drillControl.pause} />}
      className="detail-window-game"
      onClose={closeDrill}
    >
      <TimeRegion runtime={runtime} region={drillControl.region}>
      {shut ? <GateState gate={drillControl.gate} loading="Loading…" /> : <>
      {Designer && opened && track && drillAt === null && <WidgetState kind="outside" read={drillCoverage} />}
      {Designer && opened && track && drillAt !== null && <Designer
        selection={{
          kind: 'divisionDesigner',
          sourceId: track.source.id,
          revision: track.source.revision,
          branchId: track.branch.id,
          hour: drillAt,
          tag: opened.tag,
          template: opened.template,
          designer: opened.designer,
        }}
        generation={drillAt}
        active={active}
        client={runtime.client}
        onDiagnostics={onWindowDiagnostics}
        onClose={closeDrill}
      />}
      </>}
      </TimeRegion>
    </DetailWindowHost>
  </div>;
}
