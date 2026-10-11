import { createContext, useContext } from 'react';
import { createPortal } from 'react-dom';
import { Icon } from '../components/Icons';

/** Where a chart inside a widget puts its header controls (the widget frame supplies it). */
export const HeaderActionsSlot = createContext<HTMLElement | null>(null);

/** Resets a zoomed chart. Inside a widget it sits in the header, clear of the data. */
export function ZoomReset({ onReset }: { onReset: () => void }) {
  const slot = useContext(HeaderActionsSlot);
  const button = <button type="button" className={`button small ${slot ? 'quiet header-zoom-reset' : 'time-chart-reset'}`} onClick={onReset}>
    <Icon name="refresh" />Reset zoom</button>;
  return slot ? createPortal(button, slot) : button;
}
