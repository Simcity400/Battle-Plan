import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { useRegion, useRegionCoverage, useRegionHour } from '../../dashboard/widgetTime.ts';
import type { InstalledAction } from '../../families/contract.ts';
import { FamilyWidget } from '../../families/host/registry';
import { gameWindow, subscribeGameWindows } from '../../dashboard/gameWindows';
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
  const [windowLines, setWindowLines] = useState<readonly string[]>(NO_LINES);
  const onWindowDiagnostics = useCallback((lines: string[]) =>
    setWindowLines(prev => (prev.length === lines.length && prev.every((line, i) => line === lines[i]) ? prev : lines)), []);

  useEffect(() => {
    dispatch({ type: 'close' });
    setOpened(null);
  }, [track?.source.id, track?.source.revision, track?.branch.id, widget.options?.country, dispatch]);

  const handleActivate = useCallback((action: InstalledAction) => {
    if (!Designer) return;
    const info = action.meta as { designer?: 'ordinary' | 'headquarters' | null; tag?: string } | null | undefined;
    if (!info?.tag || !info.designer) return;
    setOpened({ template: action.subject, designer: info.designer, tag: info.tag });
    drillControl.show(action.subject, action.hour);
  }, [Designer, drillControl]);

  const main = <FamilyWidget
    {...props}
    family="table"
    declaration={divisionTemplateDeclaration}
    selectedKey={opened?.template ?? null}
    onActivate={Designer ? handleActivate : undefined}
    extraWarnings={windowLines}
  />;

  const closeDrill = useCallback(() => {
    dispatch({ type: 'close' });
    setOpened(null);
    setWindowLines(NO_LINES);
  }, [dispatch]);

  return <div ref={anchor} className="division-template">
    <DrillHost
      drill={drillControl}
      runtime={runtime}
      subject={drill.breakdown !== null && !!Designer && !!opened}
      mode="window"
      title="Division designer"
      day={drillAt === null ? '' : formatDateTime(drillAt)}
      canFollow
      anchor={anchor}
      onClose={closeDrill}
      main={main}
    >
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
        active={active}
        client={runtime.client}
        onDiagnostics={onWindowDiagnostics}
        onClose={closeDrill}
      />}
    </DrillHost>
  </div>;
}
