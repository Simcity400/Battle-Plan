/** The Diplomacy widgets' chart forms in the production kits and styles, with synthetic data
 * shaped as each widget builds it, mounted in a card of a chosen size. */
import { createRoot } from 'react-dom/client';
import type { ReactNode } from 'react';
import { ButterflyChart } from '../charts/butterfly';
import type { ButterflyItem } from '../charts/butterfly';
import HeatMatrix from '../charts/heat/HeatMatrix';
import FlowDiagram from '../charts/flow/FlowDiagram';
import Table, { CountryCell } from '../charts/table/Table';
import type { TableColumn } from '../charts/table/columns';
import Timeline from '../charts/timeline/Timeline';
import ProgressMeter from '../charts/meter/ProgressMeter';
import '../styles.css';
import './faction-goals/factionGoals.css';
import './diplomatic-pacts/diplomaticPacts.css';
import './war-contribution/warContribution.css';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 10; i++) await frame(); }
const host = document.createElement('div'); document.body.replaceChildren(host);
document.documentElement.dataset.motion = 'reduced';
const root = createRoot(host);
const colours = ['#4e79a7', '#f28e2b', '#e15759', '#76b7b2', '#59a14f', '#edc948'];
const names = Array.from({ length: 40 }, (_, i) => `Synthetic country with a long name ${i}`);

type Goal = { key: string; name: string; progress: number | null; status: string; term: string };
const goals: Goal[] = Array.from({ length: 24 }, (_, i) => ({ key: `g${i}`, name: `A mod faction goal with a long name ${i}`, progress: i % 5 ? (i % 10) / 10 : null,
  status: ['In progress', 'Completed', 'Cancelled'][i % 3], term: ['Short Term', 'Medium Term', 'Long Term'][i % 3] }));
const goalColumns: TableColumn<Goal>[] = [
  { key: 'goal', header: 'Goal', label: 'Goal', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name },
  { key: 'progress', header: 'Progress', label: 'Progress', numeric: true, width: 'minmax(120px, 1fr)', priority: 2, value: row => row.progress,
    renderMissing: true, render: row => <span className="faction-goal-progress"><ProgressMeter value={row.progress} state="live" />{row.progress == null ? 'Not recorded' : `${row.progress * 100}%`}</span> },
  { key: 'status', header: 'Status', label: 'Status', width: 'max-content', priority: 1, value: row => row.status, render: row => <span className="pill">{row.status}</span> },
];
const butterfly: ButterflyItem[] = ['Equipment damage', 'Occupation', 'Air damage', 'Strategic bombing', 'Ship damage', 'Convoy damage', 'Casualties contribution', 'Lend-lease sent', 'Total contribution']
  .map((label, i) => ({ id: label, label, color: '#888', segment: 'war', left: 1, right: i % 3 ? 1 : 0, leftText: '1,234.56', rightText: i % 3 ? '987.1' : 'None yet', ratioText: '',
    leftParts: colours.map((color, j) => ({ id: `l${j}`, value: j + 1, color, hatched: j === 5 })), rightParts: colours.slice(0, 3).map((color, j) => ({ id: `r${j}`, value: 3 - j, color })) }));
const matrix = names.slice(0, 12).map(name => ({ id: name, label: name }));
const flowNodes = [...names.slice(0, 6).map((name, i) => ({ id: `s${i}`, side: 'source' as const, label: name, color: colours[i], order: name })),
  ...names.slice(6, 10).map((name, i) => ({ id: `t${i}`, side: 'target' as const, label: name, color: colours[i], order: name }))];
const flowLinks = flowNodes.filter(node => node.side === 'source').map((node, i) => ({ id: `l${i}`, source: node.id, target: `t${i % 4}`, value: i + 1, color: node.color, order: `l${i}` }));
const agreements = [{ id: 'k', label: '', spans: [], heading: true, rowHeight: 30, singleLine: true, separator: false as const, identity: { label: 'A mod agreement kind with a long name · 12 in force' } },
  { id: 's', label: '', spans: [], heading: true, depth: 1, muted: true, labelStyle: 'side' as const, rowHeight: 18, singleLine: true, separator: false as const, identity: { label: 'Synthetic country docks in' } },
  ...names.slice(0, 24).map((name, i) => ({ id: `a${i}`, label: '', identity: { label: name }, depth: 1, rowHeight: 22, singleLine: true, separator: false as const, icons: false, names: false, muted: i % 3 === 0,
    spans: [{ id: 'bar', firstHour: i * 30, lastHour: i % 3 ? 1199 : i * 30 + 200, label: name, color: colours[i % 6], barHeight: 8, openEnd: true }] }))];
const lanes = [{ id: 'h', label: 'Rules', spans: [], heading: true, compact: true, singleLine: true },
  ...Array.from({ length: 6 }, (_, i) => ({ id: `r${i}`, label: `Rule ${i + 1}`, compact: true, depth: 1, names: true, icons: false,
    spans: [{ id: `r${i}a`, firstHour: 0, lastHour: 400, label: 'A mod rule with a long name' }, { id: `r${i}b`, firstHour: 401, lastHour: 1199, label: 'Another rule' }] }))];

type Member = { key: string; name: string; value: number };
const members: Member[] = Array.from({ length: 5000 }, (_, i) => ({ key: String(i), name: `Synthetic member ${i}`, value: 5000 - i }));
let drawnMembers = 0;
const memberColumns: TableColumn<Member>[] = [
  { key: 'member', header: 'Member', label: 'Member', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name, render: row => <CountryCell name={row.name} /> },
  { key: 'manpower', header: 'Manpower', label: 'Manpower contributed', numeric: true, width: 'max-content', bar: 'visible', value: row => row.value },
  { key: 'share', header: 'Share', label: 'Share of the pool', numeric: true, width: 'max-content', priority: 1, value: row => row.value / 12502500 },
];
const forms: Record<string, () => ReactNode> = {
  'faction-pooled-strength': () => <Table label="Manpower" rows={members} columns={memberColumns} rowKey={row => row.key} sort={{ column: 'manpower', direction: 'descending' }}
    onVisibleRows={rows => { drawnMembers = rows.length; }} />,
  'war-contribution': () => <div className="war-contribution"><ButterflyChart label="Contributions" items={butterfly} leftLabel="◀ Viewpoint’s side" rightLabel="Opposing side ▶"
    ratioLabel="" showRatio={false} showFlags={false} showValues={host.clientWidth >= 420} axis active /></div>,
  'faction-goals': () => <div className="faction-goals"><Table label="Goals" rows={goals} columns={goalColumns} rowKey={row => row.key}
    groupBy={row => ({ key: row.term, label: row.term })} sort={{ column: 'progress', direction: 'descending' }} /></div>,
  'diplomatic-relations': () => <HeatMatrix label="Opinions" rows={matrix} columns={matrix} revision={1} active tooltip={() => null}
    cell={(row, column) => row === column ? null : (row + column) % 5 ? { color: colours[(row + column) % 6] } : { unknown: true }} />,
  'diplomatic-pacts': () => <div className="diplomatic-pacts"><Timeline label="Diplomatic agreements" extent={[0, 1200]} playhead={900} lanes={agreements} showIcons={false} showNames={false} active revision={1} /></div>,
  'faction-upgrades': () => <div className="timeline-scroll"><Timeline label="Rules" extent={[0, 1200]} playhead={600} lanes={lanes} showIcons={false} showNames active revision={1} /></div>,
  volunteers: () => <FlowDiagram label="Volunteers" nodes={flowNodes} links={flowLinks} stretch={1} active />,
};

export const fixture = {
  async mount(form: string, width: number, height: number) {
    host.style.cssText = `width:${width}px;height:${height}px;display:flex;flex-direction:column;overflow:hidden;position:relative`;
    root.render(<div key={`${form}:${width}:${height}`} style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0, minWidth: 0 }}>{forms[form]()}</div>);
    await settle();
  },
  forms: () => Object.keys(forms),
  snapshot() {
    const card = host.firstElementChild as HTMLElement;
    const tableScroll = [...card.querySelectorAll<HTMLElement>('.chart-table')].map(element => element.scrollWidth - element.clientWidth);
    return { cardScrollWidth: card.scrollWidth, cardWidth: card.clientWidth, tableScroll,
      headers: [...card.querySelectorAll('thead th')].map(cell => cell.getAttribute('aria-label')),
      text: card.textContent?.slice(0, 120) ?? '',
      canvas: [...card.querySelectorAll('canvas')].some(canvas => canvas.width > 0 && canvas.height > 0), drawnMembers };
  },
};
declare global { interface Window { diplomacySizesFixture: typeof fixture } }
window.diplomacySizesFixture = fixture;
