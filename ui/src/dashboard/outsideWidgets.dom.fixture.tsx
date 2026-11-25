import { createRoot } from 'react-dom/client';
import '../widgets';
import type { FrameEngine } from '../engine/frame';
import type { WidgetRuntime } from './definitions';
import type { WidgetSpec } from './model';
import { PlaybackController } from './playback';
import type { EquipmentLossesReply } from '../generated/dashboard';
import { WidgetBody } from './widgets';

const engine = { every: () => ({ wake() {}, stop() {} }) } as unknown as FrameEngine;
const clock = new PlaybackController(engine);
clock.setBounds({ firstHour: 0, lastHour: 200 });

const losses: EquipmentLossesReply = {
  from: 100,
  hour: 105,
  through: 109,
  landSource: 'destruction',
  warnings: [],
  breakdown: {
    unrecordedHours: 0,
    total: { amount: { numerator: '350001', denominator: '100000' }, state: 'known', reason: null },
    parts: [{
      key: 'destruction:landCombat', name: 'Land combat losses', nameMissing: null, ordinal: 0, other: false,
      amount: { numerator: '350001', denominator: '100000' }, state: 'known', reason: null, share: 1, parentShare: 1,
      count: null, last: null, art: null, children: [],
    }],
  },
};
const identities = {
  branchId: '1', warnings: [], known: [{ firstHour: 100, lastHour: 109 }],
  countries: [{ tag: 'GER', identities: [{ slot: 1, countryId: 1, firstHour: 100, lastHour: 109 }], unresolved: [] }],
};
const groups = {
  branchId: '1', warnings: [], warStarts: [], capitulations: [], identityChanges: [], factions: [], wars: [], humans: [],
  unobserved: { humans: [], factions: [], wars: [] },
  countryIdentities: [{ tag: 'GER', identities: [{ slot: 1, countryId: 1, firstHour: 100, lastHour: 109 }] }],
};

const waiting = new Promise<never>(() => {});
const client = {
  available: true,
  openFollow: async () => {},
  product: async (request: { sourceId: string; revision: string; product: string }) => {
    const reply = (data: unknown) => ({ sourceId: request.sourceId, revision: request.revision, product: request.product, data });
    if (request.product === 'country-identities') return reply(identities);
    if (request.product === 'country-groups') return reply(groups);
    if (request.product === 'equipment-losses') return reply(losses);
    if (request.product === 'follow') return reply({ available: false, reason: 'not followed here' });
    if (request.product === 'follow-clock' || request.product === 'unfollow') return reply({ available: true });
    return waiting;
  },
  setMapViews: async () => {}, onMapStates: async () => () => {}, releaseSource: async () => {}, openSource: async () => { throw new Error('none'); },
};
const source = { id: 'outside', revision: '1', metadata: { countries: [], campaignKey: 'campaign', captureMode: 'full', branches: [{ id: '1', firstHour: 100, lastHour: 109, gaps: [] }] } };
const track = { id: 'main', roleId: 'main', label: 'main', branchIndex: 0, anchorOffset: null, axisOffset: 0, selectedTag: 'GER', selectedSlot: 1, source, branch: { id: '1', firstHour: 100, lastHour: 109, gaps: [] } };
const runtime = { tracks: [track], campaignTracks: [], defaultRoleId: null, clock, client, addWidgetBelow() {}, campaigns: [], recordings: [], requestSeek() {} } as unknown as WidgetRuntime;
const spec = (id: string, kind: string) => ({ id, kind, configVersion: 1, title: id, trackIds: ['main'], countries: { kind: 'tags', tags: ['GER'] }, time: { mode: 'follow' }, options: {} }) as unknown as WidgetSpec;

const settle = async () => { for (let turn = 0; turn < 6; turn++) await new Promise<void>(resolve => setTimeout(resolve, 10)); };
const text = (id: string) => document.getElementById(id)?.textContent ?? '';

Object.assign(window, { outsideWidgets: {
  async mount(id: 'losses', hour: number) {
    clock.seek(hour);
    const host = document.body.appendChild(document.createElement('div'));
    host.id = id;
    host.style.width = '600px';
    host.style.height = '320px';
    createRoot(host).render(<WidgetBody widget={spec(id, 'equipment-losses')} runtime={runtime} active headerSlot={null} countrySlot={null} onDiagnostics={() => {}} setOption={() => {}} />);
    await settle();
    return text(id);
  },
  async seek(id: string, hour: number) { clock.seek(hour); await settle(); return text(id); },
  async step(id: string) { clock.step(1); await settle(); return { hour: clock.getHour(), text: text(id) }; },
} });
