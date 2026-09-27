import { useLayoutEffect, useRef, useSyncExternalStore } from 'react';
import { placeTooltip } from './tooltipPlacement';
import { tooltipStore } from './tooltip';
import './charts.css';

/** Renders chart tooltips in the top layer, so no widget edge can clip them. As a
 * `role="tooltip"` popover it never hides the native map. */
export default function TooltipLayer() {
  const state = useSyncExternalStore(tooltipStore.subscribe, tooltipStore.get, tooltipStore.get);
  const ref = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    if (!state) { if (element.matches(':popover-open')) element.hidePopover(); return; }
    if (!element.matches(':popover-open')) element.showPopover();
    const bounds = element.getBoundingClientRect();
    const { x, y } = placeTooltip({ x: state.x, y: state.y }, { width: bounds.width, height: bounds.height },
      { width: document.documentElement.clientWidth, height: document.documentElement.clientHeight });
    element.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
  }, [state]);
  return <div ref={ref} popover="manual" role="tooltip" className="chart-tooltip" aria-live="off">
    {state && <>
      {state.content.icon || state.content.subtitle
        ? <div className="chart-tooltip-head">
          {state.content.icon && <img src={state.content.icon} alt="" />}
          <div><strong>{state.content.title}</strong>{state.content.subtitle && <small>{state.content.subtitle}</small>}</div>
        </div>
        : <strong>{state.content.title}</strong>}
      {state.content.rows.map((row, index) => <div key={index} className={`chart-tooltip-row${row.divider ? ' has-divider' : ''}${row.strong ? ' is-strong' : ''}${row.short ? ' is-short' : ''}`}>
        <span>{row.swatch && <i style={{ background: row.swatch }} aria-hidden="true" />}{row.label}</span>
        {row.value != null && <b className={row.tone ? `is-${row.tone}` : undefined}>{row.value}</b>}
      </div>)}
      {state.content.description && <p className="chart-tooltip-description">{state.content.description}</p>}
      {state.content.note && <small>{state.content.note}</small>}
    </>}
  </div>;
}
