import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import './index.ts';
import '../styles.css';
import type { DashboardClient, DashboardSource, ProductRequest } from '../dashboard/client.ts';
import type { WidgetRuntime } from '../dashboard/definitions.ts';
import type { BoundTrack } from '../dashboard/model.ts';
import { createWidget } from '../dashboard/model.ts';
import { PlaybackController } from '../dashboard/playback.ts';
import { WidgetBody } from '../dashboard/widgets.tsx';
import { pipelineOf } from '../dashboard/follow/useFollowed.ts';
import { preparingClient } from '../dashboard/prepare.ts';
import { laneModules } from './manifest.ts';
import type { CountryGroupReply, DivisionLogTailReply, MarketLedgerTailReply, PeaceDealsTailReply } from '../generated/dashboard.ts';

const FIRST = 1234, LAST = 1247;
const common = { branchId: 'branch', total: '1', offset: '0', missing: [], warnings: [], nextChange: null };
const tails = {
  'market-ledger-tail': { ...common, live: '1', counts: [], rows: [{ line: 'event', key: 'ledger-witness', contract: 'contract-witness',
    seller: 'AAA', buyer: 'BBB', hour: FIRST, kind: 'delivery', variant: null, equipment: 'Ledger witness', equipmentMissing: null,
    amountRaw: null, amountNotRecorded: true, reason: 'Synthetic missing amount' }] } satisfies MarketLedgerTailReply,
  'peace-deals-tail': { ...common, counts: [{ key: 'treaties', total: '1' }], rows: [{ key: 'group:peace-witness', hour: FIRST, parent: null,
    detail: { kind: 'treaty', winner: 'AAA', loser: 'BBB', whitePeace: true, bounded: false, derived: false, members: 0, terms: [], limitations: [] }, last: false }] } satisfies PeaceDealsTailReply,
  'division-log-tail': { ...common, available: true, unavailable: null, censusIncomplete: false, templates: [], counts: [{ key: 'created', total: '1' }],
    rows: [{ key: 'group:division-witness', hour: FIRST, observed: FIRST, slot: 9, tag: 'AAA', parent: null, event: null, members: 1, open: false }] } satisfies DivisionLogTailReply,
};
const groups = { branchId: 'branch', warnings: [], warStarts: [], capitulations: [], identityChanges: [], factions: [], wars: [], humans: [],
  unobserved: { humans: [], factions: [], wars: [] }, countryIdentities: [] } as unknown as CountryGroupReply;
const source = { id: 'logs', revision: 'revision', path: 'synthetic.battleplan', metadata: { captureMode: 'full', branches: [{
  id: 'branch', firstHour: FIRST, lastHour: LAST, main: true, gaps: [], joins: [], fork: null,
}], countries: [], warnings: [], buildWarnings: [], playerCountryId: null } } as unknown as DashboardSource;
const track = { id: 'main', roleId: 'primary', source, branch: source.metadata.branches[0], axisOffset: 0, selectedTag: null } as unknown as BoundTrack;

Object.assign(window, { logIsolation: async (kinds: string[], prepared: boolean) => {
  const asked: string[] = [];
  const client = { available: true, product: async (request: ProductRequest) => {
    asked.push(request.product);
    let data: unknown;
    if (request.product in tails) data = tails[request.product as keyof typeof tails];
    else if (request.product === 'country-groups') data = groups;
    else if (request.product === 'country-identities') data = { branchId: 'branch', known: [], countries: [], unknown: [], warnings: [] };
    else throw new Error(`Not recorded in fixture: ${request.product}`);
    return { sourceId: request.sourceId, revision: request.revision, product: request.product, data };
  } } as unknown as DashboardClient;
  const clock = new PlaybackController();
  clock.setBounds({ firstHour: FIRST, lastHour: LAST });
  clock.seek(FIRST);
  const runtime = { campaignTracks: [track], defaultRoleId: 'primary', tracks: [track], clock,
    client: prepared ? preparingClient(client) : client, alignment: 'anchors', nativeHidden: false } as unknown as WidgetRuntime;
  const pipeline = pipelineOf(clock, runtime.client);
  const needs = laneModules(kinds);
  pipeline.prepare(needs.families, needs.projections);
  await pipeline.whenPrepared();
  const host = document.body.appendChild(document.createElement('div'));
  host.style.cssText = 'width:1000px;height:420px;position:relative';
  const root = createRoot(host);
  const shown = [];
  try {
    for (const kind of kinds) {
      const widget = { ...createWidget(kind, 'main', kind), countries: { kind: 'all' as const }, time: { mode: 'pin' as const, windows: { main: { firstOffset: 0, lastOffset: 0 } } } };
      flushSync(() => root.render(<WidgetBody key={kind} widget={widget} runtime={runtime} />));
      for (let turn = 0; turn < 200 && !host.querySelector('tbody tr[data-row-key]'); turn++)
        await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
      shown.push({ kind, rows: [...host.querySelectorAll('tbody tr[data-row-key]')].map(row => row.getAttribute('data-row-key')),
        columns: [...host.querySelectorAll('thead th')].map(cell => cell.textContent), text: host.textContent, canvas: !!host.querySelector('canvas') });
    }
    return { asked, shown };
  } finally { root.unmount(); pipeline.close(); host.remove(); }
} });
