/** Dockyard spending's and Military factory spending's two views as the
 * widgets draw them (their own bodies, the widget frame's header with Live and
 * the view switch), from synthetic replies with long mod names, mounted in a
 * card of a chosen size. */
import { createRoot } from 'react-dom/client';
import type { ReactNode } from 'react';
import { Segmented } from '../components/Segmented';
import type { DockyardMonth, DockyardSpendingReply, FactoryMonth, MilitaryFactorySpendingReply, ProductionSpendingLeaf } from '../generated/dashboard';
import { resolveOptions } from '../dashboard/options';
import { firstHourOfMonth } from '../dashboard/calendar';
import { DOCKYARD } from './dockyard-spending/DockyardSpendingWidget';
import { PurposeCircle, PurposeMonths } from './dockyard-spending/PurposeSpending';
import { OPTIONS, VIEWS } from './dockyard-spending/definition';
import { definition } from './dockyard-spending/definition';
import { MILITARY_FACTORY } from './military-factory-spending/MilitaryFactorySpendingWidget';
import { OPTIONS as FACTORY_OPTIONS } from './military-factory-spending/definition';
import '../styles.css';
import '../dashboard/dashboard.css';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 40; i++) await frame(); }
const host = document.createElement('div'); document.body.replaceChildren(host);
document.documentElement.dataset.motion = 'reduced';
const root = createRoot(host);

const start = firstHourOfMonth(1936 * 12), months = 120, end = firstHourOfMonth(1936 * 12 + months) - 1;
const leaf = (index: number, refit: boolean): ProductionSpendingLeaf => ({
  variant: String(100 + index), typeKey: `mod_hull_${index % 6}`, name: `A mod hull type with a long name ${index % 6}`, domain: 'naval',
  rocket: false, purpose: refit ? 'refit' : 'new', unproven: [],
  names: [{ firstHour: start, name: `A mod ship design with a long name ${index}`, known: true, design: null }],
  runs: [{ firstHour: start, lastHour: end, totalRaw: String((index + 1) * (refit ? 30_000 : 900_000)), reason: null }],
});
const month = (index: number): DockyardMonth => ({
  firstHour: firstHourOfMonth(1936 * 12 + index), lastHour: firstHourOfMonth(1936 * 12 + index + 1) - 1,
  newShips: { totalRaw: String(4_000_000 + (index % 7) * 900_000), reason: null },
  refits: { totalRaw: String((index % 5) * 300_000), reason: null },
  repairs: index % 23 === 9 ? { totalRaw: null, reason: 'recordingGap' } : { totalRaw: String((index % 11) * 200_000), reason: null },
  unrecorded: { totalRaw: String((index % 3) * 100_000), reason: null }, other: { totalRaw: '0', reason: null },
  purposeUnproven: index % 3 !== 0, unresolved: index % 31 === 4,
});
const reply: DockyardSpendingReply = {
  branchId: '1', tag: 'SYN', decimals: 5, reads: [start], observations: [start], warnings: [],
  equipmentReads: [start], rocketReads: [start], equipmentObservations: [start], rocketObservations: [start],
  rocketKey: null, rocketName: null, rocketTotal: [], production: [{ firstHour: start, lastHour: end, present: true, emptyCensusHour: null }],
  leaves: Array.from({ length: 30 }, (_, index) => leaf(index, index % 4 === 0)),
  total: [{ firstHour: start, lastHour: end, totalRaw: '1', reason: null }],
  equipmentTotal: [{ firstHour: start, lastHour: end, totalRaw: '1', reason: null }],
  repairs: { hour: { totalRaw: '123456789', reason: null }, end: { totalRaw: '123456789', reason: null }, denominator: 24, since: null, parts: [
    ...Array.from({ length: 8 }, (_, index) => ({ variant: String(200 + index), typeKey: `mod_hull_${index % 3}`, name: `A mod hull type with a long name ${index % 3}`,
      domain: 'naval' as const, names: [], hour: { totalRaw: String(10_000_000 + index), reason: null }, end: { totalRaw: String(10_000_000 + index), reason: null } })),
    { variant: null, typeKey: '', name: null, domain: null, names: [], hour: { totalRaw: '43456781', reason: null }, end: { totalRaw: '43456781', reason: null } }] },
  months: Array.from({ length: months }, (_, index) => month(index)),
};

const factoryMonth = (index: number): FactoryMonth => ({
  firstHour: firstHourOfMonth(1936 * 12 + index), lastHour: firstHourOfMonth(1936 * 12 + index + 1) - 1,
  new: { totalRaw: String(9_000_000 + (index % 7) * 900_000), reason: null },
  converted: index % 23 === 9 ? { totalRaw: null, reason: 'recordingGap' } : { totalRaw: String((index % 5) * 700_000), reason: null },
  unrecorded: { totalRaw: String((index % 4) * 200_000), reason: null }, other: { totalRaw: '0', reason: null },
  purposeUnproven: index % 4 !== 0, unresolved: index % 31 === 4,
});
const factoryLeaf = (index: number, converted: boolean): ProductionSpendingLeaf => ({
  ...leaf(index, false), typeKey: `mod_airframe_${index % 6}`, name: `A mod airframe type with a long name ${index % 6}`, domain: 'air',
  purpose: converted ? 'converted' : 'new',
  names: [{ firstHour: start, name: `A mod aircraft design with a long name ${index}`, known: true, design: null }],
});
const factoryReply: MilitaryFactorySpendingReply = {
  ...reply, leaves: Array.from({ length: 30 }, (_, index) => factoryLeaf(index, index % 3 === 0)),
  months: Array.from({ length: months }, (_, index) => factoryMonth(index)),
};

const card = ({ product, name }: { product: string; name: string }, view: 'circle' | 'months', body: ReactNode) =>
  <article className={`dash-widget kind-${product}`} aria-label={name}>
  <header className="dash-widget-header">
    <div className="dash-widget-title"><h2>{name}<span className="dash-widget-summary" /></h2></div>
    <div className="dash-widget-actions"><span className="dash-widget-actions-slot">
      <Segmented value={view} options={VIEWS} label={`${name} view`} onChange={() => {}} /></span>
      <button type="button" role="switch" aria-checked={false} className="live-toggle"><span className="live-toggle-track" aria-hidden="true">
        <span className="live-toggle-knob" /></span><span className="live-toggle-label">Live</span></button></div>
  </header>
  <div className="dash-widget-content"><div className="dash-widget-scaled">{body}</div></div>
</article>;

const options = resolveOptions(OPTIONS, {});
const factoryOptions = resolveOptions(FACTORY_OPTIONS, {});
const forms: Record<string, () => ReactNode> = {
  breakdown: () => card(DOCKYARD, 'circle', <PurposeCircle kind={DOCKYARD} data={reply} shown={{ from: null, hour: end, end }} requestedHour={end} options={options}
    decimals={5} digits={0} hoursPerSecond={24} active />),
  'by-month': () => card(DOCKYARD, 'months', <PurposeMonths kind={DOCKYARD} data={reply} extent={[start, end]} playhead={null} branchStart={start}
    decimals={5} digits={0} hoursPerSecond={24} active />),
  'factory-breakdown': () => card(MILITARY_FACTORY, 'circle', <PurposeCircle kind={MILITARY_FACTORY} data={factoryReply}
    shown={{ from: null, hour: end, end }} requestedHour={end} options={factoryOptions} decimals={5} digits={0} hoursPerSecond={24} active />),
  'factory-by-month': () => card(MILITARY_FACTORY, 'months', <PurposeMonths kind={MILITARY_FACTORY} data={factoryReply} extent={[start, end]}
    playhead={null} branchStart={start} decimals={5} digits={0} hoursPerSecond={24} active />),
};

export const fixture = {
  /** The widget's minimum size at the grid's scale. */
  minimum(): [number, number] {
    return [definition.minSize.w * 75, definition.minSize.h * 44];
  },
  async mount(form: string, width: number, height: number) {
    host.style.cssText = `width:${width}px;height:${height}px;display:flex;flex-direction:column;overflow:hidden;position:relative`;
    root.render(<div key={`${form}:${width}:${height}`} style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0, minWidth: 0 }}>{forms[form]()}</div>);
    await settle();
  },
  forms: () => Object.keys(forms),
  snapshot() {
    const card = host.querySelector<HTMLElement>('.dash-widget')!;
    const bounds = card.getBoundingClientRect();
    // Header controls and legend entries must not be pushed past the card's edge.
    const clipped = [...card.querySelectorAll<HTMLElement>('.segmented, .live-toggle, .chart-legend li, .sunburst-legend li')]
      .filter(element => element.getBoundingClientRect().right > bounds.right + 1).map(element => element.className || element.tagName);
    const canvases = [...card.querySelectorAll('canvas')];
    return { cardScrollWidth: card.scrollWidth, cardWidth: card.clientWidth, clipped,
      text: card.textContent?.slice(0, 120) ?? '',
      canvas: canvases.some(canvas => canvas.width > 0 && canvas.height > 0),
      canvasInside: canvases.every(canvas => canvas.getBoundingClientRect().right <= bounds.right + 1 && canvas.getBoundingClientRect().bottom <= bounds.bottom + 1) };
  },
};
declare global { interface Window { dockyardSizesFixture: typeof fixture } }
window.dockyardSizesFixture = fixture;
