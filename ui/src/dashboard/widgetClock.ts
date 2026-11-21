import type { PlaybackController } from './playback';
import { perfLog, timePaint } from '../shell/perf.ts';

const pages = new WeakMap<PlaybackController, PlaybackController>();
export const pageClock = (clock: PlaybackController) => pages.get(clock) ?? clock;

export class WidgetFrames {
  readonly clock: PlaybackController;
  private readonly page: PlaybackController;
  private readonly widget: string;
  private readonly listeners = new Set<() => void>();
  private shown = true;
  private stop: (() => void) | null = null;

  constructor(page: PlaybackController, widget: string) {
    this.page = page;
    this.widget = widget;
    const bound = new Map<PropertyKey, unknown>();
    const subscribeFrame = this.subscribeFrame;
    this.clock = new Proxy(page, { get(target, key) {
      if (key === 'subscribeFrame') return subscribeFrame;
      const value: unknown = Reflect.get(target, key, target);
      if (typeof value !== 'function') return value;
      let made = bound.get(key);
      if (!made) bound.set(key, made = (value as (...args: unknown[]) => unknown).bind(target));
      return made;
    } });
    pages.set(this.clock, pageClock(page));
  }

  setShown(shown: boolean): void {
    if (shown === this.shown) return;
    this.shown = shown;
    this.sync();
    if (shown && this.listeners.size) this.run();
  }

  private readonly subscribeFrame = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    this.sync();
    return () => { this.listeners.delete(listener); this.sync(); };
  };

  private sync(): void {
    const wanted = this.shown && this.listeners.size > 0;
    if (wanted && !this.stop) this.stop = this.page.subscribeFrame(this.run);
    else if (!wanted && this.stop) { this.stop(); this.stop = null; }
  }

  private readonly run = (): void => {
    if (perfLog) timePaint(this.widget, this.page.frameAt, this.tell);
    else this.tell();
  };

  private readonly tell = (): void => {
    for (const listener of this.listeners) listener();
  };
}
