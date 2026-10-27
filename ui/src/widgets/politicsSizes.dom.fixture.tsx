/** The Politics widgets' forms in the production kits, columns and styles, with
 * synthetic data shaped as each widget builds it, mounted in a card of a chosen size.
 * The tables use the widgets' own column factories, the balance view its own
 * component and the civil wars their own lanes. */
import { createRoot } from 'react-dom/client';
import type { ReactNode } from 'react';
import DrillParts from '../charts/drill/DrillParts';
import LogList from '../charts/log/LogList';
import Table from '../charts/table/Table';
import Timeline from '../charts/timeline/Timeline';
import ArtFallback from '../components/ArtFallback';
import { ChipSet } from '../components/ChipSet';
import { Icon } from '../components/Icons';
import type { WidgetRuntime } from '../dashboard/definitions';
import type { BoundTrack } from '../dashboard/model';
import type { BalanceHistory, CivilWar } from '../generated/dashboard';
import BalanceCountryView, { BalanceHeadline } from './balance-of-power/BalanceCountryView';
import { balanceColumns } from './balance-of-power/BalanceTable';
import type { BalanceRow } from './balance-of-power/balanceOfPowerData';
import { driftParts } from './balance-of-power/balanceOfPowerData';
import { civilWarLanes, civilWarsAt } from './civil-wars/civilWarsData';
import { exileColumns } from './governments-in-exile/GovernmentsInExileWidget';
import type { ExileRow } from './governments-in-exile/governmentsInExileData';
import { electionColumns } from './next-elections/NextElectionsWidget';
import type { ElectionRow } from './next-elections/nextElectionsData';
import { widgetMetas } from './definitions';
import '../styles.css';
import '../charts/log/log.css';
import './event-feed/eventFeed.css';
import './peace-deals/peaceDeals.css';
import './event-choices/eventChoices.css';
import './next-elections/nextElections.css';
import './governments-in-exile/governmentsInExile.css';
import './balance-of-power/balanceOfPower.css';
import './civil-wars/civilWars.css';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 10; i++) await frame(); }
const host = document.createElement('div'); document.body.replaceChildren(host);
document.documentElement.dataset.motion = 'reduced';
const root = createRoot(host);
const tags = Array.from({ length: 60 }, (_, i) => `M${String(i).padStart(2, '0')}`);
const label = (tag: string) => ({ name: `Synthetic country with a long name ${tag}`, known: true });
const sortName = (tag: string) => label(tag).name;
const runtime = { clock: { getSnapshot: () => ({ hoursPerSecond: 0 }) } } as unknown as WidgetRuntime;
const track = { id: 'main', axisOffset: 0, branch: { id: '1', firstHour: 0, lastHour: 4000, gaps: [] } } as unknown as BoundTrack;

const elections: ElectionRow[] = tags.map((tag, i) => ({ key: tag, tag, interval: 48, next: 2000 + i * 100, termStart: 0, term: (i % 10) / 10, hoursLeft: 1000 + i * 100 }));
const exiles: ExileRow[] = tags.map((tag, i) => ({ key: tag, tag, host: tags[(i + 7) % tags.length], legitimacy: String(1_234_567 + i), manpower: 12_345 + i,
  cap: 24_000, leaders: i, readHour: 100 }));
const run = (firstHour: number, lastHour: number, value: string) => ({ firstHour, lastHour, value });
const balance = (token: string): BalanceHistory => ({ token, name: 'A mod balance of power with a long name',
  left: { token: `${token}_left`, name: 'The left side with a long name' }, right: { token: `${token}_right`, name: 'The right side with a long name' },
  ranges: [{ token: `${token}_low`, name: 'Low', min: '-100000', max: '-25000' }, { token: `${token}_mid`, name: 'A middle range with a long name', min: '-25000', max: '25000' },
    { token: `${token}_high`, name: 'High', min: '25000', max: '100000' }],
  values: [run(0, 1999, '-34000'), run(2000, 4000, '12000')], record: [run(0, 4000, '0')], range: [run(0, 1999, `${token}_low`), run(2000, 4000, `${token}_mid`)],
  trending: [run(0, 4000, `${token}_right`)] });
const balances = ['mod_balance_a', 'mod_balance_b', 'mod_balance_c'].map(balance);
const balanceRows: BalanceRow[] = tags.slice(0, 30).map((tag, i) => ({ key: tag, token: balances[i % 3].token, name: balances[i % 3].name, value: String(i * 1000 - 15_000),
  range: `${balances[i % 3].token}_mid`, trending: `${balances[i % 3].token}_right`, members: [{ holder: tag, tag }, { holder: `${tag}b`, tag: tags[(i + 1) % 60] }],
  balance: balances[i % 3], since: 100 }));
const side = (tag: string, firstHour: number, lastHour: number, parent: number | null) => ({ tags: [{ hour: 0, tag }], firstHour, lastHour, parent, existedBefore: parent === null, exit: null });
const wars: CivilWar[] = Array.from({ length: 8 }, (_, i) => ({ id: `w${i}`, firstHour: i * 300, lastHour: i * 300 + 1500, atWar: [{ firstHour: i * 300, lastHour: i * 300 + 1500 }],
  sides: [side(tags[i * 3], i * 300, i * 300 + 1500, null), side(tags[i * 3 + 1], i * 300, i * 300 + 1500, 0), side(tags[i * 3 + 2], i * 300 + 200, i * 300 + 1400, 1)],
  outcome: null }));
const warView = civilWarsAt({ wars }, 2000, { ended: true, countryIds: null, identities: null, unread: [], lastHour: 4000 }).wars;
const warLanes = civilWarLanes(warView, new Set(), { side: tag => label(tag), dated: tag => label(tag), day: hour => `Day ${hour}`, neutral: '#888', positive: '#2a2' }, 0, []).lanes;
const record = { ordinal: 0, token: 'mod_balance_a', name: 'A mod balance of power with a long name', value: '-34000', left: { token: 'l', name: 'Left' },
  right: { token: 'r', name: 'Right' }, trending: null, range: null, ranges: [],
  members: tags.slice(0, 6).map((tag, i) => ({ tag, daily: String(i - 3), weekly: String((i - 3) * 7) })),
  sources: Array.from({ length: 6 }, (_, i) => ({ token: `mod_static_${i}`, name: `A mod static modifier with a long name ${i}`, daily: String(i), weekly: String(i * 7) })) };
const drift = driftParts(record, tag => label(tag));
const options = ['Stand firm with a long option text that wraps', 'Back down', 'Ask the allies for help with a long option text'];
const choices = Array.from({ length: 20 }, (_, i) => ({ key: `c${i}`, hour: 4000 - i * 24, item: { tag: tags[i], sender: i % 2 ? tags[i + 1] : null, chosen: i % 3 === 0 ? null : i % 3 } }));
const plain = (value: number) => String(value);

const forms: Record<string, { kind: string; render: () => ReactNode }> = {
  'next-elections': { kind: 'next-elections', render: () => <div className="next-elections"><Table label="Next elections" rows={elections} rowKey={row => row.key}
    columns={electionColumns(sortName, label, true)} sort={{ column: 'days', direction: 'ascending' }} rank rowHeight={36} /></div> },
  'governments-in-exile': { kind: 'governments-in-exile', render: () => <div className="governments-in-exile"><Table label="Governments in exile" rows={exiles}
    rowKey={row => row.key} columns={exileColumns(sortName, label, true, row => row.leaders)} sort={{ column: 'manpower', direction: 'descending' }} rank rowHeight={36} /></div> },
  'balance-of-power-table': { kind: 'balance-of-power', render: () => <div className="balance-of-power"><Table label="Balances of power" rows={balanceRows}
    rowKey={row => row.key} columns={balanceColumns(sortName, label, () => 'A middle range with a long name', () => 'The right side with a long name')}
    sort={{ column: 'value', direction: 'descending' }} rank rowHeight={36} /></div> },
  'balance-of-power-country': { kind: 'balance-of-power', render: () => <div className="balance-of-power"><BalanceCountryView runtime={runtime} track={track} active
    subject={{ balances, shown: [run(0, 4000, 'mod_balance_a')] }} names={{}} balance={balances[0]} gameShown="mod_balance_a" countryName="Synthetic country"
    at={2500} limit={4000} extent={[0, 4000]} playhead={2500} playback={{ mode: 'draw-in', afterPlayhead: false, windowHours: 8760 }} unknown={[]} pending={[]}
    membersOf={() => tags.slice(0, 4).map(sortName)} pickOwner={null} /></div> },
  'balance-of-power-weekly': { kind: 'balance-of-power', render: () => <div className="balance-of-power is-pinned"><p className="chart-line-key">1 January 1941</p>
    <div className="balance-drill"><BalanceHeadline name={record.name} token={record.token} detail="Low · −0.34 · toward Right" />
      <DrillParts parts={drift.sources ?? []} format={plain} showShares={false} totalRow={{ label: 'Sources weekly', value: '+0.00105' }} />
      <p className="chart-line-key"><strong>Members</strong></p>
      <DrillParts parts={drift.members} format={plain} totalRow={{ label: 'Members weekly', value: '−0.00021' }} /></div></div> },
  'civil-wars': { kind: 'civil-wars', render: () => <div className="civil-wars-widget"><Timeline label="Civil wars" extent={[0, 4000]} playhead={2000} labelHour={2000}
    lanes={warLanes} showIcons={false} showNames={false} active revision={1} /></div> },
  'event-choices': { kind: 'event-choices', render: () => <div className="log-widget peace-deals event-choices">
    <div className="log-chips"><ChipSet label="Event choices filters" chips={[{ value: 'a', label: 'Country events' }, { value: 'b', label: 'News events' },
      { value: 'c', label: 'Awaiting choice only' }]} selected={['a', 'b']} max={3} onToggle={() => {}} /></div>
    <LogList label="Event choices" layout="card" density="comfortable" months animate={false} rows={choices} rowHeight={() => 78 + 18 * options.length}
      describe={row => row.key} renderRow={row => <article className="peace-deal event-choice">
        <div className="event-choice-heading"><span className="log-art" aria-hidden="true"><ArtFallback icon="ballot" /></span>
          <b>A mod event with a long title that needs room {row.key}</b><small>3 Mar 1938 → chosen 5 Mar 1938 · 2 days</small></div>
        <div className="event-choice-countries"><span><b>{sortName(row.item.tag)}</b></span>
          {row.item.sender && <><span>from</span><span><b>{sortName(row.item.sender)}</b></span></>}
          {row.item.chosen === null && <span className="pill accent"><span>Awaiting choice · 12 days open</span></span>}</div>
        <ol className="event-choice-options">{options.map((text, i) => <li key={text} className={`event-choice-option${row.item.chosen === i ? ' is-chosen' : ''}`}>
          {row.item.chosen === i ? <Icon name="check" /> : <i aria-hidden="true" />}<span>{String.fromCharCode(65 + i)}. {text}</span></li>)}</ol>
      </article>} /></div> },
};

export const fixture = {
  /** The form's widget minimum (its definition's minSize), at the harness's 4 x 5 = 300 x 220 scale. */
  minimum(form: string): [number, number] {
    const { w, h } = widgetMetas.find(meta => meta.kind === forms[form].kind)!.minSize;
    return [w * 75, h * 44];
  },
  async mount(form: string, width: number, height: number) {
    host.style.cssText = `width:${width}px;height:${height}px;display:flex;flex-direction:column;overflow:hidden;position:relative`;
    root.render(<div key={`${form}:${width}:${height}`} style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0, minWidth: 0 }}>{forms[form].render()}</div>);
    await settle();
  },
  forms: () => Object.keys(forms),
  snapshot() {
    const card = host.firstElementChild as HTMLElement;
    const tableScroll = [...card.querySelectorAll<HTMLElement>('.chart-table')].map(element => element.scrollWidth - element.clientWidth);
    // Controls and text rows must not be pushed past the card's right edge.
    const bounds = card.getBoundingClientRect();
    const clipped = [...card.querySelectorAll<HTMLElement>('.chip-set, .balance-headline, .event-choice-heading, .event-choice-countries, .event-choice-option, .drill-part-label, .chart-line-key')]
      .filter(element => element.getBoundingClientRect().right > bounds.right + 1).map(element => element.className);
    return { cardScrollWidth: card.scrollWidth, cardWidth: card.clientWidth, tableScroll, clipped,
      headers: [...card.querySelectorAll('[role="columnheader"]')].map(cell => cell.getAttribute('aria-label') ?? cell.textContent),
      text: card.textContent?.slice(0, 120) ?? '',
      canvas: [...card.querySelectorAll('canvas')].some(canvas => canvas.width > 0 && canvas.height > 0) };
  },
};
declare global { interface Window { sizesFixture: typeof fixture } }
window.sizesFixture = fixture;
