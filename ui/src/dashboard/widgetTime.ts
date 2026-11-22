import { createContext, useContext, useSyncExternalStore } from 'react';

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

export interface WidgetTime { heads: ActiveHeads; outside: boolean; detail: string | undefined }

export const WidgetTimeContext = createContext<WidgetTime | null>(null);

const unsubscribed = () => () => {};
const none = () => false;

export function useOutsideRecording(): { outside: boolean; detail: string | undefined } {
  const time = useContext(WidgetTimeContext);
  const reading = useSyncExternalStore(time?.heads.subscribe ?? unsubscribed, time?.heads.any ?? none, time?.heads.any ?? none);
  return { outside: !!time?.outside && reading, detail: time?.detail };
}
