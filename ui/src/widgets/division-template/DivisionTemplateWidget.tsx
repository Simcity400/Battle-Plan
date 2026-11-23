import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from 'react';
import WidgetState from '../../components/WidgetState';
import DrillHost from '../../charts/drill/DrillHost';
import { useDrill } from '../../charts/drill/useDrill';
import { formatDateTime } from '../../dashboard/calendar';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { useRegion, useRegionCoverage, useRegionHour } from '../../dashboard/widgetTime.ts';
import DeclaredTableWidget from '../../families/table/DeclaredTableWidget';
import { OPTIONS } from './definition';
import { designerSnapshot, subscribeDesignerSnapshot } from './designerSnapshot';
import { divisionTemplateDeclaration } from './divisionTemplateProjection';
import './divisionTemplate.css';

const NO_LINES: readonly string[] = [];

export default function DivisionTemplateWidget(props: WidgetProps) {
  const { widget, runtime, active } = props;
  const track = runtime.tracks.find(t => t.id === widget.trackIds[0]);
  const options = resolveOptions(OPTIONS, widget.options);
  const drillControl = useDrill(runtime, useRegion(widget));
  const drill = drillControl.state;
  const dispatch = drillControl.dispatch;
  const drillRegion = useRegion(widget, drillControl.region.action);
  const drillCoverage = useRegionCoverage(runtime, drillRegion);
  const drillAt = useRegionHour(runtime, drillRegion, active && drill.breakdown !== null);
  const snapshot = useSyncExternalStore(subscribeDesignerSnapshot, designerSnapshot, designerSnapshot);
  const anchor = useRef<HTMLDivElement>(null);
  const [opened, setOpened] = useState<{ template: string; designer: 'ordinary' | 'headquarters'; tag: string } | null>(null);
  const [windowLines, setWindowLines] = useState<readonly string[]>(NO_LINES);
  const onWindowDiagnostics = useCallback((lines: string[]) =>
    setWindowLines(prev => (prev.length === lines.length && prev.every((line, i) => line === lines[i]) ? prev : lines)), []);

  useEffect(() => {
    dispatch({ type: 'close' });
    setOpened(null);
  }, [track?.source.id, track?.source.revision, track?.branch.id, widget.options?.country, dispatch]);

  const handleActivate = useCallback((key: string, meta?: unknown, atHour?: number) => {
    if (!snapshot || atHour === undefined) return;
    const info = meta as { designer?: 'ordinary' | 'headquarters'; tag?: string } | null | undefined;
    if (!info?.tag) return;
    setOpened({ template: key, designer: info.designer ?? 'ordinary', tag: info.tag });
    drillControl.show(key, atHour);
  }, [snapshot, drillControl]);

  const main = <DeclaredTableWidget
    {...props}
    declaration={divisionTemplateDeclaration}
    selectedKey={opened?.template ?? null}
    onActivate={snapshot ? handleActivate : undefined}
    extraWarnings={windowLines}
  />;

  return <div ref={anchor} className="division-template">
    <DrillHost
      drill={drillControl}
      runtime={runtime}
      subject={drill.breakdown !== null && !!snapshot && !!opened}
      mode={options.drill === 'popout' ? 'popout' : 'inline'}
      title="Division designer"
      day={drillAt === null ? '' : formatDateTime(drillAt)}
      canFollow
      anchor={anchor}
      onClose={() => { dispatch({ type: 'close' }); setOpened(null); setWindowLines(NO_LINES); }}
      main={main}
    >
      {snapshot && opened && track && drillAt === null && <WidgetState kind="outside" read={drillCoverage} />}
      {snapshot && opened && track && drillAt !== null && snapshot({
        client: runtime.client,
        source: track.source,
        branchId: track.branch.id,
        hour: drillAt,
        tag: opened.tag,
        template: opened.template,
        designer: opened.designer,
        onDiagnostics: onWindowDiagnostics,
      }, active)}
    </DrillHost>
  </div>;
}
