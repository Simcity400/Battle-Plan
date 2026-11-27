import { useEffect, useLayoutEffect, useMemo, useRef, useSyncExternalStore } from 'react';
import { CountryFlagUrl } from '../components/CountryFlag';
import { placeTooltip } from './tooltipPlacement';
import { handleActiveTooltipKey, handleActiveTooltipWheel, tooltipStore } from './tooltip';
import { ScrollableTooltipRows, TooltipRowView } from './TooltipRows';
import './charts.css';
import { commitZone } from '../shell/commitCount';

export default function TooltipLayer() {
  const state = useSyncExternalStore(tooltipStore.subscribe, tooltipStore.get, tooltipStore.get);
  const content = useMemo(() => state?.detailRows
    ? { ...state.content, rows: [...state.content.rows, ...state.detailRows] }
    : state?.content, [state?.content, state?.detailRows]);
  const ref = useRef<HTMLDivElement>(null);
  const scrollable = Boolean(state?.content.scrollable);
  useEffect(() => {
    if (!scrollable) return;
    const maxScroll = () => {
      const element = ref.current;
      return element ? Math.max(0, element.scrollHeight - element.clientHeight) : 0;
    };
    const eventOwner = (target: EventTarget | null, client?: { readonly x: number; readonly y: number }): string | null => {
      const element = ref.current;
      const activeOwner = tooltipStore.get()?.owner ?? null;
      if (client && element && activeOwner) {
        const rect = element.getBoundingClientRect();
        if (client.x >= rect.left && client.x <= rect.right && client.y >= rect.top && client.y <= rect.bottom) {
          return activeOwner;
        }
      }
      if (!(target instanceof Element)) return null;
      return target.closest('[data-tooltip-owner]')?.getAttribute('data-tooltip-owner') ?? null;
    };
    const onWheel = (event: WheelEvent) => {
      if (event.ctrlKey || event.metaKey || event.deltaY === 0) return;
      const owner = eventOwner(event.target, { x: event.clientX, y: event.clientY });
      if (!handleActiveTooltipWheel(owner, event.deltaY, maxScroll())) return;
      event.preventDefault();
      event.stopPropagation();
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.ctrlKey || event.metaKey) return;
      const owner = eventOwner(event.target);
      if (!handleActiveTooltipKey(owner, event.key, event.altKey, maxScroll())) return;
      event.preventDefault();
      event.stopPropagation();
    };
    window.addEventListener('wheel', onWheel, { passive: false, capture: true });
    window.addEventListener('keydown', onKeyDown, { capture: true });
    return () => {
      window.removeEventListener('wheel', onWheel, true);
      window.removeEventListener('keydown', onKeyDown, true);
    };
  }, [scrollable]);
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
  return <div ref={ref} data-detail={state?.content.detail ?? state?.content.countryFlag?.nameFailure?.message} popover="manual" role="tooltip" className={`chart-tooltip${state?.content.scrollable ? ' is-scrollable' : ''}`} aria-live="off">
    {state && content && <>
      {state.content.icon || state.content.countryFlag || state.content.subtitle || state.content.pinned
        ? <div className="chart-tooltip-head">
          {state.content.countryFlag ? <CountryFlagUrl className="chart-tooltip-flag" url={state.content.countryFlag.flag}
            detail={state.content.countryFlag.flagDetail} reason={state.content.countryFlag.flagReason} pending={state.content.countryFlag.flagPending} />
            : state.content.icon && <img src={state.content.icon} alt="" />}
          <div>
            <strong>{state.content.title}</strong>
            {state.content.subtitle && <small>{state.content.subtitle}</small>}
            {state.content.pinned && <small className="chart-tooltip-pinned">{state.content.pinned}</small>}
          </div>
        </div>
        : <strong>{state.content.title}</strong>}
      {state.content.scrollable ? <ScrollableTooltipRows content={content} owner={state.owner} top={state.scrollTop ?? 0} />
        : state.content.rows.map((row, index) => <TooltipRowView key={index} row={row} />)}
      {state.content.description && <p className="chart-tooltip-description">{state.content.description}</p>}
      {state.content.note && <small>{state.content.note}</small>}
    </>}
  </div>;
}

commitZone(TooltipLayer, 'tooltips');
