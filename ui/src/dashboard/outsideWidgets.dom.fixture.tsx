import { createRoot } from 'react-dom/client';
import type { FrameEngine } from '../engine/frame';
import type { WidgetProps, WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { PlaybackController } from './playback';
import type { LossesReply, TradeFlowsReply } from '../generated/dashboard';
import TradeFlowsWidget from '../widgets/trade-flows/TradeFlowsWidget';
import EquipmentLossesWidget from '../widgets/equipment-losses/EquipmentLossesWidget';
import { registerWidgets } from './definitions';
import { definition as tradeFlows } from '../widgets/trade-flows/definition';
import { definition as equipmentLosses } from '../widgets/equipment-losses/definition';

registerWidgets([tradeFlows, equipmentLosses]);

const engine = { every: () => ({ wake() {}, stop() {} }) } as unknown as FrameEngine;
const clock = new PlaybackController(engine);
clock.setBounds({ firstHour: 0, lastHour: 200 });

const losses: LossesReply = {
  branchId: '1', ships: [], from: 100, to: 110, width: 64, exactHours: [], restarts: [], tag: 'GER', amountScale: 100_000, landSource: 'combatLog', landLowerBound: null,
  destructionRuns: [{ firstHour: 100, lastHour: 109, countedFrom: 100, withheldFrom: null }], destructionCoverage: [{ firstHour: 100, lastHour: 109, reason: null }], retainedMonths: 7, warnings: [],
  designs: [{ variant: 91001, equipment: 'mod_walkers_v2', archetype: 'mod_walkers', archetypeName: 'Walking machines', names: [{ firstHour: 100, name: 'Walker', known: true }] }],
  coverage: [{ firstHour: 100, lastHour: 109, coverage: 'logged' }],
  causes: [{ source: { kind: 'combatLog', reason: 0 }, domain: 'land', name: { key: 'mod_loss_combat', template: 'Mod combat', text: 'Mod combat' }, group: null, loss: true,
    unknown: null, unknownSpans: [], leaves: [{ design: 0, key: '0', points: [{ hour: 100, totalRaw: '200000' }, { hour: 105, totalRaw: '350001' }], exact: [], lowerBoundFrom: null }] }],
};
const trade: TradeFlowsReply = {
  branchId: '1', decimals: 5, resources: [{ id: 1, token: 'steel', ordinal: 1, name: 'Steel', icon: null }], countries: ['GER', 'ITA'],
  countryIdentities: ['GER', 'ITA'].map((tag, slot) => ({ tag, runs: [{ firstHour: 100, lastHour: 109, slot, countryId: slot + 1 }] })),
  firstHour: 100, lastHour: 109, known: [{ firstHour: 100, lastHour: 109 }], partial: [],
  flows: [{ seller: 0, buyer: 1, resource: 1, sellerIdentity: '[0,1]', buyerIdentity: '[1,2]', sellerTags: [0], buyerTags: [1], firstHours: [100], lastHours: [109],
    deliveredRaw: ['1000000'], factories: [1], convoysAssigned: [0], convoysRequired: [0], deals: [1] }],
  warnings: [],
};
Object.assign(globalThis, { __fixture: { losses, trade, groups: { branchId: '1', warnings: [], warStarts: [], capitulations: [], identityChanges: [], factions: [], wars: [], humans: [],
  unobserved: { humans: [], factions: [], wars: [] }, countryIdentities: [{ tag: 'GER', identities: [{ slot: 1, countryId: 1, firstHour: 100, lastHour: 109 }] }] } } });

const waiting = new Promise<never>(() => {});
const client = {
  available: true,
  openFollow: async () => {},
  product: async (request: { sourceId: string; revision: string; product: string }) => {
    const reply = (data: unknown) => ({ sourceId: request.sourceId, revision: request.revision, product: request.product, data });
    if (request.product === 'follow') return reply({ available: true, route: 1 });
    if (request.product === 'follow-clock' || request.product === 'unfollow') return reply({ available: true });
    return waiting;
  },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};
const source = { id: 'outside', revision: '1', metadata: { countries: [], campaignKey: 'campaign', captureMode: 'full' } };
const track = { id: 'main', roleId: 'main', label: 'main', branchIndex: 0, anchorOffset: null, axisOffset: 0, selectedTag: 'GER', source, branch: { id: '1', firstHour: 100, lastHour: 109, gaps: [] } };
const runtime = { tracks: [track], campaignTracks: [], defaultRoleId: null, clock, client, addWidgetBelow() {}, campaigns: [], recordings: [], requestSeek() {} } as unknown as WidgetRuntime;
const spec = (id: string, kind: string) => ({ id, kind, configVersion: 1, title: id, trackIds: ['main'], countries: { kind: 'tags', tags: ['GER'] }, time: { mode: 'follow' }, options: {} }) as unknown as WidgetSpec;
const props = (widget: WidgetSpec) => ({ widget, runtime, active: true, headerSlot: null, countrySlot: null, onDiagnostics() {}, setOption() {} }) as unknown as WidgetProps;

const settle = async () => { for (let turn = 0; turn < 3; turn++) await new Promise<void>(resolve => setTimeout(resolve, 0)); };
const text = (id: string) => document.getElementById(id)?.textContent ?? '';
const widgets = { trade: TradeFlowsWidget, losses: EquipmentLossesWidget };

Object.assign(window, { outsideWidgets: {
  async mount(id: keyof typeof widgets, hour: number) {
    clock.seek(hour);
    const Widget = widgets[id];
    const host = document.body.appendChild(document.createElement('div'));
    host.id = id;
    createRoot(host).render(<Widget {...props(spec(id, id === 'trade' ? 'trade-flows' : 'equipment-losses'))} />);
    await settle();
    return text(id);
  },
  async seek(id: string, hour: number) { clock.seek(hour); await settle(); return text(id); },
  async step(id: string) { clock.step(1); await settle(); return { hour: clock.getHour(), text: text(id) }; },
} });
