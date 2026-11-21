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

export async function glance<T>(act: <R>(expression: string) => Promise<R>, page: string): Promise<T> {
  await act(`mount(${JSON.stringify(page)}, 'page')`);
  for (let turn = 0; turn < 30 && await act<boolean>('scroll()'); turn++);
  return act<T>(`glanced(${JSON.stringify(page)})`);
}

export async function glanceAll<T>(act: <R>(expression: string) => Promise<R>): Promise<[string, T][]> {
  await act('start()');
  const glanced: [string, T][] = [];
  for (const page of await act<string[]>('pages()')) glanced.push([page, await glance<T>(act, page)]);
  return glanced;
}

export interface RecordedWindow {
  from: number; head: number; frames: number; elapsed: number; messages: number; delivered: number; lastHour: number | null;
  commits: Record<string, number>; why: Record<string, Record<string, number>>;
  asked: { product: string; params: Record<string, unknown>; from: number | null; head: number | null; ms: number; served: boolean }[];
  misses: string[]; layout: number; measure: number; paints: Record<string, number[]>; shown: Record<string, number | null>;
  states: { id: string; kind: string; state: string | null; text: string | null }[];
}

export const PLAYED = ['Politics', 'Overview', 'Army'];

export async function playWindow(act: <R>(expression: string) => Promise<R>, page: string, until?: number): Promise<RecordedWindow> {
  await act(`mount(${JSON.stringify(page)}, 'play', 1440)`);
  return act<RecordedWindow>(`play(${JSON.stringify(page)}${until === undefined ? '' : `, ${until}`})`);
}

export type RecordedValues = Record<string, { kind: string; text: string; shown: number | null }>;
export const pageValues = (act: <R>(expression: string) => Promise<R>): Promise<RecordedValues> => act<RecordedValues>('values()');
