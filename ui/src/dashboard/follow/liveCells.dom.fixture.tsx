/** A panel's live cells in a real browser: a table whose rows are a pushed panel's structure and whose numbers are its
 * followed cells, fed messages as the service writes them; what it paints, and how many commits it took. */
import { Profiler, useEffect, useMemo, useState } from 'react';
import { createRoot } from 'react-dom/client';
import Table from '../../charts/table/Table';
import type { TableColumn } from '../../charts/table/Table';
import { CELLS, followedCells, liveSection } from './liveCells';
import { fakeTransport, message } from './messages.fixture';
import { FollowStore } from './store';
import '../../styles.css';

document.documentElement.dataset.motion = 'reduced';
const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 8; i++) await frame(); }

const transport = fakeTransport();
const store = new FollowStore(transport, run => requestAnimationFrame(() => run()));
const handle = store.acquire({ panel: 'synthetic-power', branchId: '1' });
let commits = 0;
let follow = CELLS + 1;

interface Row { key: string }
const columns: TableColumn<Row>[] = [
  { key: 'name', header: 'Country', label: 'Country', width: '160px', value: row => row.key },
  { key: 'power', header: 'Power', label: 'Power', width: '120px', numeric: true, value: () => null, paint: value => `${value} pp` },
];
const rowKey = (row: Row) => row.key;

/** The panel's rows render (a commit only when the structure lands); their power is painted from the followed cells. */
function Panel() {
  const [, setVersion] = useState(0);
  useEffect(() => store.watch(handle.entry, () => setVersion(version => version + 1)), []);
  const live = useMemo(() => followedCells({ section: () => liveSection(handle.entry.panel?.data), group: id => store.cellGroup(id),
    subscribe: changed => store.watchLive(handle.entry, changed) }), []);
  const rows = ((handle.entry.panel?.data as { rows?: string[] } | undefined)?.rows ?? []).map(key => ({ key }));
  return <Table label="Synthetic panel" rows={rows} columns={columns} rowKey={rowKey} sort={{ column: 'name', direction: 'ascending' }} live={live} />;
}

const element = document.body.appendChild(document.createElement('div'));
element.style.cssText = 'width:420px;height:400px;display:flex;flex-direction:column;position:relative';
createRoot(element).render(<Profiler id="panel" onRender={() => { commits++; }}><Panel /></Profiler>);

const fixture = {
  commits: () => commits,
  /** A structure push naming `rows`, its cells followed under a new id; the id. */
  async structure(rows: string[]) {
    follow++;
    const data = { rows, live: { groups: [{ table: 2, columns: [2], keys: rows, follow }], rows, kinds: [{ id: 'power', op: 'value', scale: 1 }],
      cells: rows.flatMap((_, row) => [row, 0, 1, 0, row, 0]) } };
    transport.deliver(new TextEncoder().encode(`PNL1${JSON.stringify({ panels: [{ id: handle.entry.id, hour: 100, data, refused: null }] })}`));
    await settle();
    return follow;
  },
  /** The current group's answer: every row's power (`changed`: only these rows), a row left out of a whole answer has none. */
  async cells(powers: Record<string, number>, changed = false) {
    const keys = Object.keys(powers);
    transport.deliver(message(100, [{ id: follow, answer: { keys: keys.map(BigInt), columns: 1, values: keys.map(key => BigInt(powers[key]!)), changed } }]));
    await settle();
  },
  async notRecorded(reason: string) {
    transport.deliver(message(100, [{ id: follow, answer: { notRecorded: reason } }]));
    await settle();
  },
  /** Each mounted row's key, painted text and its title. */
  rows: () => [...element.querySelectorAll<HTMLElement>('tbody tr:not([aria-hidden="true"])')]
    .sort((a, b) => Number(a.ariaRowIndex) - Number(b.ariaRowIndex)).map(row => {
      const text = row.querySelector<HTMLElement>('.table-live');
      return { key: row.dataset.tableRow, text: text?.textContent ?? null, title: text?.title ?? '' };
    }),
};

declare global { interface Window { liveCells: typeof fixture } }
window.liveCells = fixture;
