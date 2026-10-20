/** Diplomacy widgets mounted whole in the production hooks and styles, reading synthetic
 * replies from a client the test controls. Every reply arrives asynchronously; name reads
 * for the pooled-strength checks wait until the test releases them.
 *
 * Country RRR passes to another holder at hour 24, within one 720-hour look-sampling
 * interval of the branch start: its earlier holder (renamed QQQ from 24) is magenta and
 * "Earlier holder", its later holder blue and "Later holder". SSS is green throughout. */
import type { ComponentType } from 'react';
import { createRoot } from 'react-dom/client';
import { registerWidgets } from '../dashboard/definitions';
import type { WidgetProps } from '../dashboard/definitions';
import { createWidget } from '../dashboard/model';
import type { CountrySelection, WidgetSpec, WidgetTime } from '../dashboard/model';
import type { OptionValue } from '../dashboard/options';
import { PlaybackController } from '../dashboard/playback';
import type { DashboardClient, ProductReply, ProductRequest } from '../dashboard/client';
import type { BranchMetadata, CountryGroupReply, CountryStyle, FactionDetailReply } from '../generated/dashboard';
import { definition as pooledDefinition } from './faction-pooled-strength/definition';
import FactionPooledStrengthWidget from './faction-pooled-strength/FactionPooledStrengthWidget';
import { definition as influenceDefinition } from './faction-influence/definition';
import FactionInfluenceWidget from './faction-influence/FactionInfluenceWidget';
import { definition as relationsDefinition } from './diplomatic-relations/definition';
import DiplomaticRelationsWidget from './diplomatic-relations/DiplomaticRelationsWidget';
import { definition as warDefinition } from './war-contribution/definition';
import WarContributionWidget from './war-contribution/WarContributionWidget';
import { definition as volunteersDefinition } from './volunteers/definition';
import VolunteersWidget from './volunteers/VolunteersWidget';
import '../styles.css';
import './faction-pooled-strength/factionPooledStrength.css';
import './faction-influence/factionInfluence.css';
import './diplomatic-relations/diplomaticRelations.css';
import './war-contribution/warContribution.css';
import './volunteers/volunteers.css';

registerWidgets([pooledDefinition, influenceDefinition, relationsDefinition, warDefinition, volunteersDefinition]);
const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 12; i++) await frame(); }
const host = document.createElement('div'); document.body.replaceChildren(host);
host.style.cssText = 'width:600px;height:400px;display:flex;flex-direction:column;overflow:hidden;position:relative';
document.documentElement.dataset.motion = 'reduced';
const errors: string[] = [];
let notes: string[] = [];
const root = createRoot(host, { onUncaughtError: error => { errors.push(String(error instanceof Error ? error.stack ?? error.message : error)); } });

const branch: BranchMetadata = { id: '1', sessionId: '1', firstHour: 0, lastHour: 200, parentId: null, parentHour: null, sampleBoundary: 'Sample origin unknown', gaps: [] };
const span = { firstHour: 0, lastHour: 200 };
const COVERED = { ...span, complete: true };
const faction = (id: string, tags: string[]) => ({ id, name: `Synthetic faction ${id}`, nameReason: null, recordedName: id, ...span,
  names: [{ ...span, name: `Synthetic faction ${id}`, reason: null }], members: tags.map(tag => ({ tag, ...span })), leaders: [], incomplete: [] });
const steady = (tag: string, slot: number) => ({ tag, identities: [{ ...span, slot, countryId: slot * 10 }] });
const groups: CountryGroupReply = { branchId: '1', warnings: [], humans: [], wars: [], warStarts: [], capitulations: [], identityChanges: [],
  factions: [faction('f1', ['AAA', 'BBB']), faction('f2', ['RRR', 'SSS'])],
  countryIdentities: [steady('AAA', 1), steady('BBB', 2), steady('SSS', 7),
    { tag: 'RRR', identities: [{ firstHour: 0, lastHour: 23, slot: 5, countryId: 50 }, { firstHour: 24, lastHour: 200, slot: 6, countryId: 60 }] },
    { tag: 'QQQ', identities: [{ firstHour: 24, lastHour: 200, slot: 5, countryId: 50 }] }],
  unobserved: { humans: [], factions: [], wars: [] } };

/** The groups replies of other synthetic recordings (by source revision): an hourly Human
 * players group with no humans whose membership was not recorded from 40 to 60, one with
 * no humans proven throughout, and a reply answered while the branch is still being
 * prepared, before any holder or membership is read. */
const noHumans = { ...groups, humans: [], unobserved: { humans: [], factions: [], wars: [] } };
const groupsByRevision: Record<string, CountryGroupReply> = { '1': groups,
  gap: { ...noHumans, unobserved: { humans: [{ firstHour: 40, lastHour: 60 }], factions: [], wars: [] } }, empty: noHumans,
  partial: { ...noHumans, countryIdentities: [], ...{ coverage: { firstHour: 0, lastHour: -1, complete: false }, pending: [{ firstHour: 0, lastHour: 200 }] } } as CountryGroupReply };

/** Each country's recorded look at an hour. */
const MAGENTA: [number, number, number] = [250, 0, 250], BLUE: [number, number, number] = [0, 0, 255], GREEN: [number, number, number] = [0, 200, 0];
function styleOf(tag: string, hour: number): CountryStyle | null {
  if (tag === 'AAA') return { tag, name: 'Synthetic country AAA', color: [10, 20, 30] };
  if (tag === 'RRR') return hour < 24 ? { tag, name: 'Earlier holder', color: MAGENTA } : { tag, name: 'Later holder', color: BLUE };
  if (tag === 'QQQ') return { tag, name: 'Earlier holder', color: MAGENTA };
  if (tag === 'SSS') return { tag, name: 'Synthetic SSS', color: GREEN };
  return null; // The recording has no name for BBB.
}
const account = (tag: string, raw: string) => ({ ordinal: tag === 'RRR' ? 0 : 1, country: tag === 'RRR' ? 6 : 7, tag, currentMember: true, influence: [], projection: [],
  influenceShareRaw: raw, projectionShareRaw: '0', projectionRank: 0, initiativeRaw: '0', contributionRaw: '0', contributionGain: [], previousWarScoreRaw: '0', upgrades: [], completed: [] });
const details = (hour: number, id: string) => ({ branchId: '1', hour, faction: id, warnings: [], refusals: [], ...(id === 'f1' ? {
  manpower: [{ ordinal: 0, tag: 'AAA', value: 300 }, { ordinal: 1, tag: 'BBB', value: 100 }], manpowerTotal: 400, manpowerUnlocked: true,
  resources: [{ ordinal: 0, token: 'mod_oil', name: 'Synthetic oil', icon: null, raw: '100000', reason: null }],
  researchUnlocked: false, researchBaseRaw: null, researchCountries: null, doctrineFolders: null } : { accounts: [account('RRR', '40000'), account('SSS', '30000')] }) } as unknown as FactionDetailReply);
const share = (key: string, tag: string, slot: number, firstHour: number, lastHour: number, shareRaw: string) => ({ key, tag, slot, countryId: slot * 10, retained: [],
  member: [{ firstHour, lastHour }], runs: [{ firstHour, lastHour, shareRaw, startsSegment: false }] });
const influence = { accounts: [share('old', 'RRR', 5, 0, 23, '50000'), share('new', 'RRR', 6, 24, 200, '40000'), share('sss', 'SSS', 7, 0, 200, '30000')],
  unobserved: [], pending: [], coverage: COVERED, channels: [], warnings: [] };
const opinion = (value: number) => [{ ...span, value, startsSegment: false }];
const observation = { ...span, viewpointTag: 'SSS', viewpointSide: ['SSS'], opposingSide: ['RRR'], attackersAreViewpoint: true, representative: null,
  name: { text: 'Synthetic war', known: true, reason: null, key: null } };
const war = { id: '1', ...span, startedBy: false, endedBy: false, observations: [observation], casualties: [], participants: [], capitulations: [] };
const score = (tag: string, side: string, value: string) => ({ tag, side, equipmentDamage: value, occupation: '0', airDamage: '0', strategicBombing: '0', shipDamage: '0',
  convoyDamage: '0', casualties: '0', unknownCasualties: '0', lendLeaseSent: '0', lendLeaseReceived: '0', score: '0', participation: value });
const wars = (reads: { hour: number }[] | undefined) => ({ branchId: '1', available: true, viewpoints: [{ tag: 'SSS', ...span }], warnings: [], unobserved: [],
  capitulationsUnobserved: [], countryNames: [], wars: [war], ...(reads ? { current: reads.map(read => ({ hour: read.hour, available: true, wars: [war], warnings: [],
    contributions: [{ viewpointSide: ['SSS'], opposingSide: ['RRR'], reason: null, participants: [score('SSS', 'viewpoint', '100000'), score('RRR', 'opponent', '100000')] }] })) } : {}) });
const aidLink = (key: string, sender: string, senderSlot: number, firstHour: number, lastHour: number) => ({ key, sender, senderSlot, senderCountryId: senderSlot * 10,
  host: 'SSS', hostSlot: 7, hostCountryId: 70, runs: [{ firstHour, lastHour, divisions: 3, startsSegment: false }] });

/** Every request the widgets make, and the name reads still waiting to be answered. */
const requests: ProductRequest[] = [];
let held: (() => void)[] = [];
let holding = true;
const reply = <T,>(request: ProductRequest, data: T): ProductReply<T> => ({ sourceId: request.sourceId, revision: request.revision, product: request.product, data });
const later = () => new Promise<void>(resolve => setTimeout(resolve, 5));
const client: DashboardClient = { available: true,
  openSource: async () => { throw new Error('unused'); }, releaseSource: async () => {}, setMapViews: async () => {}, onMapStates: () => () => {},
  // Preparation is still running for a partial reply: its status never settles while mounted.
  preparationStatus: (_request: unknown, signal: AbortSignal) => new Promise((_resolve, reject) =>
    signal.addEventListener('abort', () => reject(new DOMException('Request cancelled', 'AbortError')), { once: true })),
  // Prepared products arrive through the prepared query, answered here as the same replies.
  async preparedQuery<T>(source: { id: string; revision: string }, product: string, params: unknown, options: { signal: AbortSignal }): Promise<T> {
    return (await client.product<T>({ sourceId: source.id, revision: source.revision, product, params }, options.signal)).data;
  },
  async product<T>(request: ProductRequest, signal: AbortSignal): Promise<ProductReply<T>> {
    requests.push(request);
    await later();
    const params = request.params as { hour: number; tags: string[]; size?: string; faction?: string; view?: string; reads?: { hour: number }[] };
    if (request.product === 'country-groups') return reply(request, groupsByRevision[request.revision] as T);
    if (request.product === 'faction-details') return reply(request, details(params.hour, params.faction!) as T);
    if (request.product === 'faction-state') return reply(request, influence as T);
    if (request.product === 'diplomacy-state') return reply(request, (params.view === 'pair'
      ? { forward: opinion(50), reverse: opinion(-30), unobserved: [], first: null, second: null, coverage: COVERED, pending: [], warnings: [] }
      : { hour: params.hour, owners: [], unresolved: 0, snapshot: false, coverage: COVERED, pending: [], warnings: [] }) as T);
    if (request.product === 'wars') return reply(request, wars(params.reads) as T);
    if (request.product === 'volunteers-expeditionaries') return reply(request, { tallies: [], unobserved: [], warnings: [],
      links: [aidLink('old', 'RRR', 5, 0, 23), aidLink('renamed', 'QQQ', 5, 24, 200), aidLink('new', 'RRR', 6, 24, 200)] } as T);
    if (request.product === 'country-flags') return reply(request, { branchId: '1', hour: params.hour, size: params.size, flags: [],
      unavailable: params.tags.map(tag => ({ tag, reason: 'No synthetic flag.' })), warnings: [] } as T);
    if (request.product === 'country-styles') {
      const styles = { branchId: '1', hour: params.hour, warnings: [], styles: params.tags.flatMap(tag => { const style = styleOf(tag, params.hour); return style ? [style] : []; }) };
      if (holding) await new Promise<void>((resolve, reject) => { held.push(resolve); signal.addEventListener('abort', () => reject(new DOMException('Request cancelled', 'AbortError')), { once: true }); });
      return reply(request, styles as T);
    }
    throw new Error(`No synthetic ${request.product}`);
  } } as unknown as DashboardClient & { product: DashboardClient['product'] };

const clock = new PlaybackController();
clock.setBounds({ firstHour: 0, lastHour: 200 });
const runtimeFor = (revision: string) => ({ tracks: [{ id: 'track', axisOffset: 0, selectedTag: 'AAA', source: { id: 'synthetic', revision, metadata: { campaignKey: 'campaign-a', countries: [] } }, branch }],
  campaignTracks: [], client, clock, addWidgetBelow() {} } as unknown as WidgetProps['runtime']);
let runtime = runtimeFor('1');
const widgets: Record<string, ComponentType<WidgetProps>> = { 'faction-pooled-strength': FactionPooledStrengthWidget, 'faction-influence': FactionInfluenceWidget,
  'diplomatic-relations': DiplomaticRelationsWidget, 'war-contribution': WarContributionWidget, volunteers: VolunteersWidget };
const pin = (hour: number): WidgetTime => ({ mode: 'pin', windows: { track: { firstOffset: hour, lastOffset: hour } } });
let mounted = 0;
const render = (kind: string, widget: WidgetSpec, key: string) => {
  const Widget = widgets[kind];
  root.render(<div key={key} style={{ display: 'flex', flexDirection: 'column', flex: 1, minHeight: 0 }}>
    <Widget widget={widget} runtime={runtime} active mapOptions={{} as WidgetProps['mapOptions']} headerSlot={null} onDiagnostics={warnings => { notes = warnings; }} /></div>);
};
const near = (data: Uint8ClampedArray, index: number, [r, g, b]: readonly number[]) =>
  data[index + 3] > 200 && Math.abs(data[index] - r) <= 30 && Math.abs(data[index + 1] - g) <= 30 && Math.abs(data[index + 2] - b) <= 30;

export const fixture = {
  /** The pooled-strength widget at a pool and pinned hour, kept mounted across calls. */
  async mount(pool: string, hour: number) {
    const widget: WidgetSpec = { ...createWidget('faction-pooled-strength', 'track', 'pooled'), time: pin(hour), options: { pool, faction: 'f1' } };
    render('faction-pooled-strength', widget, 'pooled');
    await settle();
  },
  /** A widget mounted afresh, with name reads answered as they come. */
  async show(kind: string, settings: { countries?: CountrySelection; time?: WidgetTime | number; options?: Record<string, OptionValue>; recording?: string }) {
    holding = false;
    runtime = runtimeFor(settings.recording ?? '1');
    const base = createWidget(kind as WidgetSpec['kind'], 'track', kind);
    const time = typeof settings.time === 'number' ? pin(settings.time) : settings.time ?? base.time;
    render(kind, { ...base, time, countries: settings.countries ?? base.countries, options: settings.options ?? {} }, `${kind}:${++mounted}`);
    for (let i = 0; i < 6; i++) { await settle(); await later(); }
  },
  async release() { holding = false; const waiting = held; held = []; for (const resolve of waiting) resolve(); await settle(); },
  reset() { requests.length = 0; },
  members: () => [...host.querySelectorAll('tbody tr')].map(row => row.querySelector('td')?.textContent?.trim() ?? ''),
  /** What the widget shows (loading states by their spoken label), else why it shows nothing. */
  text: () => `${[...host.querySelectorAll('[role="status"][aria-label]')].map(item => item.getAttribute('aria-label')).join(' ')}${host.textContent || errors.join(' | ')} [${notes.join(' | ')}]`,
  flagRequests: () => requests.filter(request => request.product === 'country-flags').length,
  /** The hours a country's names or colours were read at, in request order. */
  styleHours: (tag: string) => requests.filter(request => request.product === 'country-styles' && (request.params as { tags: string[] }).tags.includes(tag))
    .map(request => (request.params as { hour: number }).hour),
  /** The tags of each request for a product. */
  requestTags: (product: string) => requests.filter(request => request.product === product).map(request => (request.params as { tags: string[] | null }).tags),
  /** Whether any drawn canvas pixel has (about) this colour. */
  drawn(colour: 'magenta' | 'blue' | 'green') {
    const rgb = { magenta: MAGENTA, blue: BLUE, green: GREEN }[colour];
    return [...host.querySelectorAll('canvas')].some(canvas => {
      if (!canvas.width || !canvas.height) return false;
      const data = canvas.getContext('2d')!.getImageData(0, 0, canvas.width, canvas.height).data;
      for (let index = 0; index < data.length; index += 4) if (near(data, index, rgb)) return true;
      return false;
    });
  },
  /** Each legend entry's text and swatch colour. */
  legend: () => [...host.querySelectorAll('.chart-legend li')].map(item => [item.textContent?.trim() ?? '',
    getComputedStyle(item.querySelector('i') ?? item).backgroundColor]),
  labels: () => [...host.querySelectorAll('canvas[aria-label]')].map(canvas => canvas.getAttribute('aria-label')),
  /** Clicks the table row that reads `text`. */
  async clickRow(text: string) {
    const row = [...host.querySelectorAll<HTMLElement>('tbody tr')].find(item => item.textContent?.includes(text));
    row?.click();
    for (let i = 0; i < 6; i++) { await settle(); await later(); }
    return !!row;
  },
};
declare global { interface Window { widgetsFixture: typeof fixture } }
window.widgetsFixture = fixture;
