import { useParityRows } from '../../dashboard/parityRows';
import { useCallback, useMemo, useRef } from 'react';
import WidgetState from '../../components/WidgetState';
import { useContextMenu } from '../../components/ContextMenu';
import { PreparedKey } from '../../charts/PreparedKey';
import HeatChart from '../../charts/heat/HeatChart';
import { jumpItem } from '../../charts/jump';
import { hatchStyle } from '../../charts/hatch';
import { mix } from '../../shell/appearance/color';
import type { WidgetProps } from '../../dashboard/definitions';
import { resolveOptions } from '../../dashboard/options';
import { useAir } from '../../dashboard/useAirSuperiority';
import { OPTIONS } from './definition';
import './airSuperiorityTimeline.css';
export default function AirSuperiorityTimelineWidget(props: WidgetProps) {
  const options = resolveOptions(OPTIONS, props.widget.options);
  const air = useAir(props, options, false);
  const menu = useContextMenu('Air superiority timeline');
  const onMenu = useCallback((hour: number, x: number, y: number) => {
    if (air.track) menu.open(x, y, [jumpItem(props.runtime, air.track.id, hour)]);
  }, [air.track, menu.open, props.runtime]);
  const legend = useMemo(() => [...air.legend, ...(options.look === 'split' ? [] : [{ id: 'contested', label: 'Contested (small lead)', color: mix(air.tokens.surface, air.tokens.text, 0.2) }]),
    ...(air.mode.theatres.length ? [{ id: 'filtered', label: 'Outlined = outside selected theatres' }] : [])],
    [air.legend, air.mode.theatres.length, options.look, air.tokens.surface, air.tokens.text]);
  const tooltip = useCallback((row: { id: string }, period: { firstHour: number } | null, hour: number) => air.tooltip(Number(row.id), period?.firstHour ?? hour), [air.tooltip]);
  const root = useRef<HTMLDivElement>(null);
  useParityRows(root, () => ({ kind: 'chart', label: 'Air superiority daily values', complete: !!air.data,
    hour: air.hour, total: air.data?.view === 'timeline' ? air.data.rows.length : 0,
    rows: air.data?.view === 'timeline' ? air.data.rows : [] }));
  if (air.needsViewpoint) return <WidgetState kind="empty" icon="plane" message="Choose a dashboard country or a viewpoint in the widget settings." />;
  if (!air.track) return <WidgetState kind="empty" icon="plane" message="Load a campaign to see its air superiority." />;
  if (!air.window || air.beforeBranch) return <WidgetState kind="unavailable" message="This time is outside the recording." />;
  if (air.error) return <WidgetState kind="error" detail={air.error} onRetry={air.retry} />;
  if (!air.data || !air.projectedTimeline || !air.labelCatalogue) return <WidgetState kind="loading" label="Loading air superiority…" />;
  if (!air.data.available) return air.data.view === 'timeline' && air.data.rowsApproximate
    ? <WidgetState kind="loading" label="Air superiority is still being prepared." /> : <WidgetState kind="unavailable" />;
  if (!air.projectedTimeline.rows.length) return <div>{air.data.view === 'timeline' && <PreparedKey approximate={air.data.approximate} pending={air.data.pending} />}
    <WidgetState kind="empty" message={air.data.view === 'timeline' && air.data.rowsApproximate ? 'No air regions in the sampled reads.' : 'No air regions match these filters.'} /></div>;
  return <div ref={root} className="air-timeline">
    {air.data.view === 'timeline' && air.data.rowsApproximate && <p className="chart-line-key" role="status">Most-active regions and historical side identities are approximate.</p>}
    <div className="air-pattern-key" aria-label="Data states">
      <span><i aria-hidden="true" style={{ background: air.tokens.surface }} />No activity</span>
      <span><i aria-hidden="true" style={{ background: `color-mix(in srgb, ${air.tokens.error} ${air.tokens.gapOpacity * 100}%, ${air.tokens.surface})` }} />Not recorded</span>
      <span><i aria-hidden="true" style={hatchStyle(air.tokens)} />Unknown</span>
    </div>
    <div className="air-timeline-chart"><HeatChart label="Air superiority timeline" rows={air.projectedTimeline.rows} periods={air.projectedTimeline.periods} labelCatalogue={air.labelCatalogue}
      extent={air.extent} playhead={air.playhead} branchStart={air.track.branch.firstHour} legend={legend} afterPlayhead={props.widget.time.mode !== 'follow'} look={options.look === 'split' ? 'split' : 'margin'}
      active={props.active} revision={air.hoverRevision} hoursPerSecond={air.hoursPerSecond}
      approximate={air.data.view === 'timeline' ? air.data.approximate : undefined} pending={air.data.view === 'timeline' ? air.data.pending : undefined}
      tooltip={tooltip} onContextMenu={onMenu} /></div>
    {menu.menu}
  </div>;
}
