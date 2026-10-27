/** The Economy widgets' forms in the production kits and styles, with synthetic data
 * shaped as each widget builds it (the same markup, classes, column widths and
 * priorities), mounted in a card of a chosen size. */
import { createRoot } from 'react-dom/client';
import type { ReactNode } from 'react';
import { ButterflyChart } from '../charts/butterfly';
import type { ButterflyItem } from '../charts/butterfly';
import FlowDiagram from '../charts/flow/FlowDiagram';
import HeatChart from '../charts/heat/HeatChart';
import LogList from '../charts/log/LogList';
import Table, { CellBar, CountryCell } from '../charts/table/Table';
import type { TableColumn } from '../charts/table/columns';
import { ChipSet } from '../components/ChipSet';
import { Segmented } from '../components/Segmented';
import { Icon } from '../components/Icons';
import { EquipmentCell } from '../dashboard/EquipmentCell';
import '../styles.css';
import '../charts/log/log.css';
import './resource-balance/resourceBalance.css';
import './fuel-branches/fuelBranches.css';
import './equipment-stockpile/equipmentStockpile.css';
import './lend-lease-balance/lendLeaseBalance.css';
import './lend-lease-agreements/lendLeaseAgreements.css';
import './international-market/internationalMarket.css';
import './event-feed/eventFeed.css';
import './market-ledger/marketLedger.css';
import './production-licences/productionLicences.css';
import './industrial-organisations/industrialOrganisations.css';
import './equipment-designs/equipmentDesigns.css';
import './air-superiority-timeline/airSuperiorityTimeline.css';
import './convoy-health/convoyHealth.css';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 10; i++) await frame(); }
const host = document.createElement('div'); document.body.replaceChildren(host);
document.documentElement.dataset.motion = 'reduced';
const root = createRoot(host);
const colours = ['#4e79a7', '#f28e2b', '#e15759', '#76b7b2', '#59a14f', '#edc948'];
const names = Array.from({ length: 40 }, (_, i) => `Synthetic country with a long name ${i}`);
const designs = Array.from({ length: 40 }, (_, i) => `A mod equipment design with a long name ${i}`);
const caption = (label: string, value: string, options: readonly { value: string; label: string }[]) => <div className="stockpile-caption-line">
  <span>1 January 1941</span><Segmented value={value} options={options} label={label} onChange={() => {}} /></div>;

type Agreement = { key: string; partner: string; from: string; equipment: string; more: number; schedule: string; delivered: string; lost: string; last: string; since: string };
const agreements: Agreement[] = Array.from({ length: 24 }, (_, i) => ({ key: `a${i}`, partner: names[i], from: names[i + 1], equipment: designs[i], more: i % 3,
  schedule: i % 2 ? 'Monthly 1,200' : 'Production share 20%', delivered: '12,345.5', lost: i % 4 ? '120' : '—', last: '4 May 1941', since: '1 January 1939' }));
const country = <T,>(key: keyof T & string, header: string, priority?: number, width = 'minmax(6em, 1.2fr)'): TableColumn<T> => ({ key, header, label: header, priority,
  width, value: row => String(row[key]), render: row => <CountryCell name={String(row[key])} /> });
const agreementColumns: TableColumn<Agreement>[] = [
  { key: 'partner', header: 'From → to', label: 'From → to', width: 'minmax(var(--lend-lease-country-min, var(--table-name-min)), 1.4fr)', value: row => row.from,
    render: row => <span className="lend-lease-pair"><CountryCell name={row.from} /><span className="lend-lease-to">→ <CountryCell name={row.partner} /></span></span> },
  { key: 'equipment', header: 'Equipment', label: 'Equipment', width: 'minmax(calc(var(--table-name-min) + 3em), 1.4fr)', value: row => row.equipment,
    render: row => <EquipmentCell name={row.equipment} detail={row.more ? `+${row.more}` : null} /> },
  { key: 'schedule', header: 'Schedule', label: 'Schedule', width: 'minmax(120px, 1fr)', priority: 1, value: row => row.schedule },
  { key: 'delivered', header: 'Delivered', label: 'Delivered', numeric: true, width: 'max-content', value: row => row.delivered },
  { key: 'lost', header: 'Lost', label: 'Lost', numeric: true, width: 'max-content', priority: 2, value: row => row.lost },
  { key: 'last', header: 'Last delivery', label: 'Last delivery', numeric: true, width: 'max-content', priority: 3, value: row => row.last },
  { key: 'since', header: 'Since', label: 'Since', numeric: true, width: 'max-content', priority: 4, value: row => row.since },
];
type Contract = { key: string; seller: string; buyer: string; equipment: string; price: string; convoys: string; collected: string; deliveries: string };
const contracts: Contract[] = Array.from({ length: 24 }, (_, i) => ({ key: `c${i}`, seller: names[i], buyer: names[i + 2], equipment: designs[i],
  price: '20.5 CIC', convoys: '2 / 4', collected: '1,234.25 CIC', deliveries: String(i) }));
const contractColumns: TableColumn<Contract>[] = [
  country<Contract>('seller', 'Seller', undefined, 'minmax(4em, 1fr)'), country<Contract>('buyer', 'Buyer', undefined, 'minmax(4em, 1fr)'),
  { key: 'equipment', header: 'Equipment', label: 'Equipment', width: 'minmax(var(--table-name-min), 1.4fr)', value: row => row.equipment,
    render: row => <EquipmentCell name={row.equipment} detail="+1" /> },
  { key: 'price', header: 'Price', label: 'Agreed price', numeric: true, width: 'max-content', priority: 1, value: row => row.price },
  { key: 'convoys', header: 'Convoys', label: 'Convoys assigned', numeric: true, width: 'max-content', priority: 3, value: row => row.convoys },
  { key: 'collected', header: 'Collected', label: 'Payment collected', numeric: true, width: 'max-content', value: row => row.collected },
  { key: 'deliveries', header: 'Deliveries', label: 'Completed deliveries', numeric: true, width: 'max-content', priority: 4, value: row => row.deliveries },
];
type Offer = { key: string; seller: string; equipment: string; amount: bigint; level: string };
const offers: Offer[] = Array.from({ length: 24 }, (_, i) => ({ key: `o${i}`, seller: names[i], equipment: designs[i], amount: BigInt(i * 1000 + 50), level: 'Normal' }));
const offerColumns: TableColumn<Offer>[] = [
  country<Offer>('seller', 'Seller', undefined, 'minmax(4em, 1fr)'),
  { key: 'equipment', header: 'Equipment', label: 'Equipment', width: 'minmax(var(--table-name-min), 1.4fr)', value: row => row.equipment, render: row => <EquipmentCell name={row.equipment} /> },
  { key: 'amount', header: 'On market', label: 'On market', numeric: true, bar: 'visible', width: 'minmax(90px, max-content)', value: row => row.amount,
    render: row => <CellBar value={row.amount} maximum={24000n}>{row.amount.toString()}</CellBar> },
  { key: 'level', header: 'Price level', label: 'Price level', width: 'max-content', priority: 1, value: row => row.level },
];
type Mio = { key: string; name: string; size: number; funds: string; traits: number; policy: string };
const mios: Mio[] = Array.from({ length: 12 }, (_, i) => ({ key: `m${i}`, name: `A mod industrial organisation with a long name ${i}`, size: 12 - i, funds: '1,234.56789',
  traits: i, policy: i % 2 ? 'A mod policy with a long name · Cooldown 12 days' : 'None' }));
const mioColumns: TableColumn<Mio>[] = [
  { key: 'organisation', header: 'Organisation', label: 'Organisation', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name,
    render: row => <span className="stockpile-equipment"><span className="mio-icon" /><span><b>{row.name}</b></span></span> },
  { key: 'size', header: 'Size', label: 'Size', numeric: true, width: 'max-content', value: row => row.size },
  { key: 'funds', header: 'Funds', label: 'Funds', numeric: true, width: 'max-content', value: row => row.funds },
  { key: 'traits', header: 'Traits', label: 'Traits unlocked', numeric: true, width: 'max-content', priority: 2, value: row => row.traits },
  { key: 'policy', header: 'Policy', label: 'Policy', width: 'minmax(120px, 1.4fr)', priority: 1, value: row => row.policy },
];
type Design = { key: string; name: string; type: string; firstSeen: string; seenObsolete: string };
const designRows: Design[] = Array.from({ length: 30 }, (_, i) => ({ key: `d${i}`, name: designs[i], type: 'A mod equipment type', firstSeen: i % 3 ? '4 May 1940' : 'By 1 January 1936', seenObsolete: i % 2 ? '1 June 1941' : 'By 1 June 1941' }));
const designColumns = (obsolete: boolean): TableColumn<Design>[] => [
  { key: 'design', header: 'Design', label: 'Design', width: 'minmax(var(--table-name-min), 2fr)', value: row => row.name, render: row => <EquipmentCell name={row.name} detail={row.type} /> },
  { key: 'firstSeen', header: 'First seen', label: 'First seen', numeric: true, width: 'max-content', priority: obsolete ? 1 : undefined, value: row => row.firstSeen },
  ...obsolete ? [{ key: 'seenObsolete', header: 'Seen obsolete', label: 'Seen obsolete', numeric: true, width: 'max-content', value: (row: Design) => row.seenObsolete }] : [],
];
const butterfly: (ButterflyItem & { id: string })[] = names.slice(0, 10).map((label, i) => ({ id: label, label, color: colours[i % 6], segment: label,
  left: 10 - i, right: i + 1, leftText: `${(10 - i) * 1234}.5`, rightText: `${(i + 1) * 987}`, ratioText: '' }));
const flowNodes = [...names.slice(0, 6).map((name, i) => ({ id: `s${i}`, side: 'source' as const, label: name, color: colours[i], order: name })),
  ...names.slice(6, 10).map((name, i) => ({ id: `t${i}`, side: 'target' as const, label: name, color: colours[i], order: name }))];
const flowLinks = flowNodes.filter(node => node.side === 'source').map((node, i) => ({ id: `l${i}`, source: node.id, target: `t${i % 4}`, value: i + 1, color: node.color, order: `l${i}` }));
const periods = Array.from({ length: 60 }, (_, i) => ({ firstHour: i * 24, lastHour: i * 24 + 23, readHour: i % 17 === 5 ? null : i * 24 + 23 }));
const heatRows = Array.from({ length: 14 }, (_, r) => ({ id: `r${r}`, label: `A mod sea region with a long name ${r}`,
  cells: periods.map((_, p) => (p + r) % 7 === 0 ? { parts: [] } : r === 13 ? { parts: [], unknown: true } : { parts: [{ id: 'e', label: 'Efficiency', color: colours[0], share: ((p * 7 + r) % 10) / 10 }] }) }));
const ledger = Array.from({ length: 40 }, (_, i) => ({ key: `e${i}`, hour: 1000 - i * 12, item: { title: `Delivery to ${names[i]}`, detail: `${names[i + 1]} → ${names[i]} · ${designs[i]}`, figure: '1,200' } }));
const COLUMNS = 'calc(var(--space-6) + var(--space-2) + var(--space-1)) minmax(0, 1fr) auto';

const forms: Record<string, () => ReactNode> = {
  'fuel-branches': () => <div className="fuel-branches-host"><div className="resource-balance fuel-branches"><ol className="resource-bars" aria-label="Fuel by branch">
    {['Army', 'Air force', 'Navy'].map((name, i) => <li key={name} role="button" tabIndex={0}>
      <span className="resource-name"><span>{name}</span></span>
      <span className="resource-track fuel-branch-track"><span className="resource-bar"><i style={{ width: `${70 - i * 20}%`, background: colours[0] }} />
        <i style={{ width: `${10 + i * 5}%`, background: '#e15759', marginLeft: 1 }} /></span>
        <small className="fuel-branch-unmet">unmet<span> 1,234.5</span></small></span>
      <b><span className="fuel-branch-received">12,345.67</span><span className="fuel-branch-requested"> / 23,456.78</span></b>
    </li>)}</ol></div></div>,
  'lend-lease-balance': () => <div className="lend-lease-balance-host"><div className="lend-lease-balance">
    {caption('Measure', 'industry', [{ value: 'industry', label: 'Industry' }, { value: 'fuel', label: 'Fuel' }])}
    <ButterflyChart label="Lend-lease" items={butterfly} slots={10} leftLabel="◀ Received" rightLabel="Sent ▶" ratioLabel="" showRatio={false} active /></div></div>,
  'lend-lease-agreements': () => <div className="lend-lease-agreements-host"><div className="lend-lease-agreements">
    {caption('Direction', 'sent', [{ value: 'sent', label: 'Sent' }, { value: 'received', label: 'Received' }])}
    <div className="lend-lease-agreements-table"><Table label="Agreements" rows={agreements} columns={agreementColumns} rowKey={row => row.key}
      sort={{ column: 'delivered', direction: 'descending' }} rowHeight={48} muted={row => row.key === 'a3'} /></div></div></div>,
  'international-market': () => <div className="international-market"><div className="international-market-table"><Table label="Contracts" rows={contracts} columns={contractColumns}
    rowKey={row => row.key} sort={{ column: 'collected', direction: 'descending' }} rowHeight={48} /></div></div>,
  'international-market-inventory': () => <div className="international-market"><div className="international-market-table"><Table label="Inventory" rows={offers} columns={offerColumns}
    rowKey={row => row.key} sort={{ column: 'amount', direction: 'descending' }} rowHeight={48} /></div></div>,
  'market-ledger': () => <div className="market-ledger-host"><div className="log-widget market-ledger">
    <div className="log-chips"><ChipSet label="Kinds" chips={['New contracts', 'Requests posted', 'Deliveries', 'Payments', 'Contracts removed'].map(label => ({ value: label, label }))}
      selected={['Deliveries']} max={5} onToggle={() => {}} /></div>
    <LogList label="Ledger" rows={ledger} density="comfortable" months animate={false} columns={COLUMNS} describe={row => row.item.title}
      renderRow={row => <><span className="log-art"><span className="event-feed-icon"><Icon name="anchor" /></span></span>
        <span className="log-text market-ledger-text"><b>{row.item.title}</b><small>{row.item.detail}</small></span>
        <span className="market-ledger-figure">{row.item.figure}</span></>} /></div></div>,
  'production-licences': () => <div className="production-licences-host"><div className="production-licences">
    <FlowDiagram label="Licences" nodes={flowNodes} links={flowLinks} stretch={1} active /></div></div>,
  'industrial-organisations': () => <div className="industrial-organisations"><Table label="Organisations" rows={mios} columns={mioColumns} rowKey={row => row.key}
    sort={{ column: 'size', direction: 'descending' }} rowHeight={48} /></div>,
  'equipment-designs': () => <div className="equipment-designs"><div className="equipment-designs-body">
    <div className="equipment-designs-table" style={{ height: 200 }}><Table label="Current" rows={designRows.slice(0, 10)} columns={designColumns(false)} rowKey={row => row.key}
      sort={{ column: 'firstSeen', direction: 'descending' }} rowHeight={48} /></div>
    <details className="equipment-designs-obsolete" open><summary>Obsolete designs (20)</summary>
      <div className="equipment-designs-table" style={{ height: 200 }}><Table label="Obsolete" rows={designRows.slice(10)} columns={designColumns(true)} rowKey={row => row.key}
        sort={{ column: 'seenObsolete', direction: 'descending' }} rowHeight={48} /></div></details></div></div>,
  'convoy-health': () => <div className="convoy-health-host"><div className="air-timeline convoy-health">
    <div className="air-pattern-key"><span><i style={{ background: '#fff' }} />No bad route</span><span><i />Not recorded</span><span><i />Efficiency not recorded</span></div>
    <div className="air-timeline-chart"><HeatChart label="Convoys" rows={heatRows} periods={periods} extent={[0, 60 * 24 - 1]} playhead={null} branchStart={0}
      legend={[{ id: 'full', label: 'Convoy efficiency (0–100%)', color: colours[0] }]} look="margin" active revision={1} tooltip={() => null} onPick={() => {}} /></div></div></div>,
};

/** Each form's widget minimum (its definition's minSize), at the harness's 4 x 5 = 300 x 220 scale. */
const MINIMUM: Record<string, [number, number]> = { 'international-market': [6, 6], 'international-market-inventory': [6, 6],
  'fuel-branches': [4, 5], 'industrial-organisations': [4, 5], 'equipment-designs': [4, 5], 'convoy-health': [4, 5] };

export const fixture = {
  minimum(form: string): [number, number] {
    const [w, h] = MINIMUM[form] ?? [4, 6];
    return [w * 75, h * 44];
  },
  async mount(form: string, width: number, height: number) {
    host.style.cssText = `width:${width}px;height:${height}px;display:flex;flex-direction:column;overflow:hidden;position:relative`;
    root.render(<div key={`${form}:${width}:${height}`} style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0, minWidth: 0 }}>{forms[form]()}</div>);
    await settle();
  },
  forms: () => Object.keys(forms),
  snapshot() {
    const card = host.firstElementChild as HTMLElement;
    const tableScroll = [...card.querySelectorAll<HTMLElement>('.chart-table')].map(element => element.scrollWidth - element.clientWidth);
    // Controls and text rows must not be pushed past the card's right edge.
    const bounds = card.getBoundingClientRect();
    const clipped = [...card.querySelectorAll<HTMLElement>('.segmented, .stockpile-caption-line, .chip-set, .resource-bars > li, .fuel-branch-received')]
      .filter(element => element.getBoundingClientRect().right > bounds.right + 1).map(element => element.className);
    return { cardScrollWidth: card.scrollWidth, cardWidth: card.clientWidth, tableScroll, clipped,
      headers: [...card.querySelectorAll('thead th')].map(cell => cell.getAttribute('aria-label')),
      text: card.textContent?.slice(0, 120) ?? '',
      canvas: [...card.querySelectorAll('canvas')].some(canvas => canvas.width > 0 && canvas.height > 0) };
  },
};
declare global { interface Window { economySizesFixture: typeof fixture } }
window.economySizesFixture = fixture;
