import { useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { CELL, rowKeys } from './message.ts';
import { fakeTransport, message } from './messages.fixture.ts';
import type { Spec } from './messages.fixture.ts';
import { BarCell, paintArmy, paintMeasure, textCell } from './painters.ts';
import type { TextCell } from './painters.ts';
import { FollowStore } from './store.ts';
import { useFollow } from './useFollow.ts';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 4; i++) await frame(); }

const transport = fakeTransport();
const passes: number[] = [];
const store = new FollowStore(transport, run => requestAnimationFrame(() => {
  const started = performance.now();
  run();
  passes.push(performance.now() - started);
}));
const renders = { measure: 0, army: 0, far: 0 };

const MEASURE = { kind: 'series', series: 'country-measure', measure: 'politicalPower', tags: ['GER', 'SOV'] };
const ARMY = { kind: 'top', table: 'ArmyTotals', column: 'owner_divisions', n: 10 };
const FAR = { kind: 'series', series: 'country-measure', measure: 'commandPower', tags: ['ITA'] };

function Measure({ query, count, tags }: { query: Record<string, unknown>; count: 'measure' | 'far'; tags: string[] }) {
  renders[count]++;
  const host = useRef<HTMLDivElement>(null);
  const cells = useRef<TextCell[]>([]);
  useFollow(store, query, host, entry => paintMeasure(entry.answer, cells.current, 5, 0));
  return <div ref={host} data-widget={count} style={{ minHeight: '20px' }}>
    {tags.map((tag, at) => <span key={tag} data-tag={tag} ref={element => { if (element) cells.current[at] = textCell(element); }} />)}
  </div>;
}

function Army() {
  renders.army++;
  const host = useRef<HTMLDivElement>(null);
  const rows = useRef<{ bar: BarCell; value: TextCell }[]>([]);
  const { structure } = useFollow(store, ARMY, host, entry => paintArmy(entry.answer, rows.current, 0, 0), rowKeys);
  const keys = structure ? structure.split(',') : [];
  rows.current.length = keys.length;
  return <div ref={host} data-widget="army" style={{ minHeight: '20px' }}>
    {keys.map((key, at) => <div key={key} data-key={key}>
      <div className="bar" ref={bar => { if (bar) rows.current[at] = { ...rows.current[at]!, bar: new BarCell(bar.style) }; }} />
      <span ref={value => { if (value) rows.current[at] = { ...rows.current[at]!, value: textCell(value) }; }} />
    </div>)}
  </div>;
}

const SWAP = { kind: 'series', series: 'country-measure', measure: 'stability', tags: ['FRA'] };
function Swapping({ loaded }: { loaded: boolean }) {
  const [element, setElement] = useState<Element | null>(null);
  const target = useMemo(() => ({ current: element }), [element]);
  useFollow(store, SWAP, target, () => {});
  return loaded ? <section ref={setElement} data-widget="swap" style={{ minHeight: '20px' }} /> : <div ref={setElement} data-widget="swap" style={{ minHeight: '20px' }} />;
}

const CONTENTS = { kind: 'series', series: 'country-measure', measure: 'warSupport', tags: ['ITA'] };
function Contents({ loaded }: { loaded: boolean }) {
  const [element, setElement] = useState<Element | null>(null);
  const target = useMemo(() => ({ current: element }), [element]);
  useFollow(store, CONTENTS, target, () => {});
  return <div ref={setElement} data-widget="contents" style={{ display: 'contents' }}>
    {loaded ? <section style={{ minHeight: '20px' }} /> : <p style={{ minHeight: '20px' }}>Loading</p>}
  </div>;
}

const host = document.body.appendChild(document.createElement('div'));
const root = createRoot(host);
const swapRoot = createRoot(document.body.insertBefore(document.createElement('div'), host));
const contentsRoot = createRoot(document.body.insertBefore(document.createElement('div'), host));

function idOf(query: Record<string, unknown>): number {
  const line = [...transport.sent].reverse().find(sent => sent.startsWith('follow ') && sent.endsWith(JSON.stringify(query)));
  if (!line) throw new Error(`${JSON.stringify(query)} is not followed`);
  return Number(line.split(' ')[1]);
}

const series = (values: bigint[]): Spec => ({ keys: values.map((_, at) => BigInt(at + 1)), columns: 1, values });

export const fixture = {
  async mount() {
    root.render(<><Measure query={MEASURE} count="measure" tags={MEASURE.tags} /><Army /><div style={{ height: '20000px' }} /><Measure query={FAR} count="far" tags={FAR.tags} /></>);
    await settle();
  },
  async swap(loaded: boolean) {
    swapRoot.render(<Swapping loaded={loaded} />);
    await settle();
  },
  async contents(loaded: boolean | null) {
    contentsRoot.render(loaded === null ? null : <Contents loaded={loaded} />);
    await settle();
  },
  renders: () => ({ ...renders }),
  sent: () => [...transport.sent],
  async deliver(hour: number, power: [number, number], army: [number, number][]) {
    const answers = [
      { id: idOf(MEASURE), answer: series(power.map(value => BigInt(value) * 100_000n)) },
      { id: idOf(ARMY), answer: { keys: army.map(([key]) => BigInt(key)), columns: 1, values: army.map(([, value]) => BigInt(value)), states: army.map(() => CELL.value) } },
    ];
    if (transport.sent.some(sent => sent.endsWith(JSON.stringify(FAR)))) answers.push({ id: idOf(FAR), answer: series([BigInt(hour) * 100_000n]) });
    transport.deliver(message(hour, answers));
    await settle();
  },
  text: (widget: string) => [...document.querySelectorAll(`[data-widget="${widget}"] span`)].map(span => span.textContent).join(' '),
  bars: () => [...document.querySelectorAll('[data-widget="army"] .bar')].map(bar => (bar as HTMLElement).style.width).join(' '),
  async scrollToFar() {
    document.querySelector('[data-widget="far"]')!.scrollIntoView();
    await settle();
  },
  async scrollToTop() {
    window.scrollTo(0, 0);
    await settle();
  },
  async receiveTimes(rounds: number) {
    const ids = Array.from({ length: 50 }, (_, at) => store.acquire({ receive: at }).entry.id);
    const messages = Array.from({ length: rounds }, (_, round) => message(round, ids.map((id, at) => ({ id, answer: {
      keys: Array.from({ length: 16 }, (_, row) => BigInt(row + at)), columns: 1,
      values: Array.from({ length: 16 }, (_, row) => BigInt((row + round) * 100_000)), names: Array.from({ length: 16 }, (_, row) => `T${row}`) } }))));
    await settle();
    const times: number[] = [];
    for (const bytes of messages) {
      const started = performance.now();
      transport.deliver(bytes);
      times.push(performance.now() - started);
      if (times.length % 10 === 0) await frame();
    }
    return { times, bytes: messages[0]!.length };
  },
  async paintMany(rounds: number) {
    const spans: HTMLSpanElement[] = [];
    const ids: number[] = [];
    const many = document.body.appendChild(document.createElement('div'));
    for (let at = 0; at < 500; at++) {
      const handle = store.acquire({ many: at });
      ids.push(handle.entry.id);
      const span = many.appendChild(document.createElement('span'));
      spans.push(span);
      const cell = [textCell(span)];
      store.painter(handle.entry, entry => paintMeasure(entry.answer, cell, 5, 0));
    }
    const messages = Array.from({ length: rounds }, (_, round) => message(round, ids.map((id, at) => ({ id, answer: series([BigInt(at + round) * 100_000n]) }))));
    await settle();
    passes.length = 0;
    const delivered: number[] = [];
    for (const bytes of messages) {
      const started = performance.now();
      transport.deliver(bytes);
      delivered.push(performance.now() - started);
      await frame();
      await frame();
    }
    return { passes: [...passes], delivered, last: spans[499]!.textContent, bytes: message(0, ids.map((id, at) => ({ id, answer: series([BigInt(at) * 100_000n]) }))).length };
  },
  async paintTable(rows: number, rounds: number) {
    const handle = store.acquire({ table: rows });
    const table = document.body.appendChild(document.createElement('table'));
    const cells = Array.from({ length: rows }, (_, row) => {
      const tr = table.appendChild(document.createElement('tr'));
      tr.appendChild(document.createElement('td')).textContent = `T${row}`;
      return textCell(tr.appendChild(document.createElement('td')));
    });
    store.painter(handle.entry, entry => paintMeasure(entry.answer, cells, 5, 0));
    const answer = (round: number) => ({ id: handle.entry.id, answer: { keys: cells.map((_, row) => BigInt(row + 1)), columns: 1,
      values: cells.map((_, row) => BigInt(row * 7 + round) * 100_000n), names: cells.map((_, row) => `T${row}`) } });
    const messages = Array.from({ length: rounds }, (_, round) => message(round, [answer(round)]));
    await settle();
    passes.length = 0;
    for (const bytes of messages) {
      transport.deliver(bytes);
      await frame();
      await frame();
    }
    return { passes: [...passes], last: table.rows[rows - 1]!.cells[1]!.textContent, bytes: messages[0]!.length };
  },
};
(window as unknown as { followFixture: typeof fixture }).followFixture = fixture;
