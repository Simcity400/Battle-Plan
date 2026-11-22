import { createContext, useContext, useSyncExternalStore } from 'react';
import type { WidgetSpec } from './model';

export class ActiveHeads {
  private readonly heads = new Set<object>();
  private readonly listeners = new Set<() => void>();

  set(head: object, active: boolean): void {
    const was = this.heads.size > 0;
    if (active) this.heads.add(head);
    else this.heads.delete(head);
    if (was === this.heads.size > 0) return;
    for (const listener of [...this.listeners]) listener();
  }

  readonly any = (): boolean => this.heads.size > 0;

  readonly subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => { this.listeners.delete(listener); };
  };
}

export const SETTINGS_DETAIL = 'Choose another time in widget settings.';

export interface ReadCoverage { outside: boolean; outsideDetail?: string }

export const outsideDetail = (widget: Pick<WidgetSpec, 'time'>, hoverHour: number | null): string | undefined =>
  widget.time.mode !== 'follow' && hoverHour === null ? SETTINGS_DETAIL : undefined;

export const followingWidget = (widget: WidgetSpec): WidgetSpec => widget.time.mode === 'follow' ? widget : { ...widget, time: { mode: 'follow' } };

export interface WidgetTime { heads: ActiveHeads; outside: boolean; detail: string | undefined; reads: Pick<WidgetSpec, 'time' | 'trackIds'> }

export const readsHostTime = (host: WidgetTime | null, widget: Pick<WidgetSpec, 'time' | 'trackIds'>, hoverHour: number | null): boolean =>
  !!host && hoverHour === null && widget.trackIds[0] === host.reads.trackIds[0]
    && (widget.time === host.reads.time || JSON.stringify(widget.time) === JSON.stringify(host.reads.time));

export const WidgetTimeContext = createContext<WidgetTime | null>(null);

const unsubscribed = () => () => {};
const none = () => false;

export function useOutsideRecording(read?: ReadCoverage): { outside: boolean; detail: string | undefined } {
  const time = useContext(WidgetTimeContext);
  const reading = useSyncExternalStore(time?.heads.subscribe ?? unsubscribed, time?.heads.any ?? none, time?.heads.any ?? none);
  if (read) return { outside: read.outside, detail: read.outsideDetail };
  return { outside: !!time?.outside && reading, detail: time?.detail };
}
