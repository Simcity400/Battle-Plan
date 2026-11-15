/** The node side of `recordedPage.dom.fixture.tsx`: a page of the browser tests set to the app's viewport as it recorded the
 * replies, and acted on through the fixture. */
import type { NativePage } from '../../../tools/native-page.ts';

type CdpModule = { cdpSession(url: string): Promise<{ send(method: string, params?: Record<string, unknown>): Promise<unknown>; close(): void }> };

export const RECORDED_FIXTURE = '/ui/src/test-support/recordedPage.dom.fixture.tsx';

export async function recordedPage(page: NativePage): Promise<<T>(expression: string) => Promise<T>> {
  const act = <T,>(expression: string) => page.evaluate<T>(`window.recordedPage.${expression}`);
  const viewport = await act<{ width: number; height: number; dpr: number }>('viewport()');
  const { cdpSession } = await import(new URL('../../../tools/e2e/cdp.mjs', import.meta.url).href) as CdpModule;
  const session = await cdpSession(page.page);
  try { await session.send('Emulation.setDeviceMetricsOverride', { width: viewport.width, height: viewport.height, deviceScaleFactor: viewport.dpr, mobile: false }); }
  finally { session.close(); }
  return act;
}

/** `page` at its mid-recording mark, scrolled a screen at a time to its bottom as the sweep's glance did. */
export async function glance<T>(act: <R>(expression: string) => Promise<R>, page: string): Promise<T> {
  await act(`mount(${JSON.stringify(page)}, 'page')`);
  for (let turn = 0; turn < 30 && await act<boolean>('scroll()'); turn++);
  return act<T>(`glanced(${JSON.stringify(page)})`);
}

/** The run replayed as the app lived it, on one client: its opening page, then every page it glanced at, so a page
 * played next holds what the app held. Returns each page's glance. */
export async function glanceAll<T>(act: <R>(expression: string) => Promise<R>): Promise<[string, T][]> {
  await act('start()');
  const glanced: [string, T][] = [];
  for (const page of await act<string[]>('pages()')) glanced.push([page, await glance<T>(act, page)]);
  return glanced;
}

/** What a window played on the recorded replies gives (`recordedPage.dom.fixture.tsx`, `play`). */
export interface RecordedWindow {
  from: number; head: number; frames: number; elapsed: number; messages: number; delivered: number; lastHour: number | null;
  commits: Record<string, number>; why: Record<string, Record<string, number>>;
  asked: { product: string; params: Record<string, unknown>; from: number | null; head: number | null; ms: number; served: boolean }[];
  misses: string[]; layout: number; measure: number; paints: Record<string, number[]>; shown: Record<string, number | null>;
  states: { id: string; kind: string; state: string | null; text: string | null }[];
}

/** The pages the quick tier plays at 1,440 h/s, in its order. */
export const PLAYED = ['Politics', 'Overview', 'Army'];

/** `page`'s 1,440 h/s window of the run, played as the app played it, stopped at `until` (an hour of the window) if given. */
export async function playWindow(act: <R>(expression: string) => Promise<R>, page: string, until?: number): Promise<RecordedWindow> {
  await act(`mount(${JSON.stringify(page)}, 'play', 1440)`);
  return act<RecordedWindow>(`play(${JSON.stringify(page)}${until === undefined ? '' : `, ${until}`})`);
}

/** What each widget of the page shows now, by widget id (`recordedPage.dom.fixture.tsx`, `values`). */
export type RecordedValues = Record<string, { kind: string; text: string; shown: number | null }>;
export const pageValues = (act: <R>(expression: string) => Promise<R>): Promise<RecordedValues> => act<RecordedValues>('values()');
