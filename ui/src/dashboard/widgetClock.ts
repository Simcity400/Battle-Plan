import type { PlaybackController } from './playback';
import { perfLog, timePaint } from '../shell/perf.ts';

const pages = new WeakMap<PlaybackController, PlaybackController>();
export const pageClock = (clock: PlaybackController) => pages.get(clock) ?? clock;

/** The page's clock as one widget's components see it: the same clock in every respect but its frame signal, which
 * reaches the widget's listeners only while the widget is on screen, and once more the moment it comes back, so what
 * they paint shows the head at once. A widget off screen does no work a frame, whatever its painters do. In a measured
 * run the widget's listeners are timed together as its painting (`paint` of the frame record). */
export class WidgetFrames {
  /** What the widget's components are given as `runtime.clock`. */
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
    // Methods are bound to the page clock, once each, so a component comparing or keying on them sees one value.
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

  /** Whether the widget is on screen: off, its listeners hear no frame; back on, each hears one at once. */
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

  /** Held on the page clock while a listener of an on-screen widget waits for frames. */
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
