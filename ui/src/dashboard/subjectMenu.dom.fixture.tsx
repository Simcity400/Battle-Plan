import { createRoot } from 'react-dom/client';
import '../widgets/index';
import type { FrameEngine } from '../engine/frame';
import type { WarsReply } from '../generated/dashboard';
import { resolveWidgetActiveView, widgetOptionFields, widgetViewOptions } from '../widgets/manifest';
import type { WidgetRuntime } from './definitions';
import type { DashboardTemplate, WidgetSpec } from './model';
import type { OptionValue } from './options';
import { OptionDraft } from './optionDraft';
import { PlaybackController } from './playback';
import WidgetCountryControl from './WidgetCountryControl';

const engine = { every: () => ({ wake() {}, stop() {} }) } as unknown as FrameEngine;
const clock = new PlaybackController(engine);
clock.setBounds({ firstHour: 0, lastHour: 200 });

const wars: WarsReply = {
  branchId: '1', available: true, viewpoints: [{ tag: 'GER', firstHour: 0, lastHour: 200 }], warnings: [], unobserved: [], capitulationsUnobserved: [], countryNames: [],
  wars: [{ id: '7', firstHour: 10, lastHour: 200, startedBy: true, endedBy: false, capitulations: [],
    participants: [{ tag: 'GER', side: 'attacker', firstHour: 10, lastHour: 200, joinedBy: true, leftBy: false }, { tag: 'POL', side: 'defender', firstHour: 10, lastHour: 200, joinedBy: true, leftBy: false }],
    observations: [{ firstHour: 10, lastHour: 200, viewpointTag: 'GER', viewpointSide: ['GER'], opposingSide: ['POL'], attackersAreViewpoint: true, representative: null,
      name: { text: 'German-Polish War', known: true, reason: null, key: null } }] }],
} as unknown as WarsReply;
clock.seek(wars.wars[0].observations[0].firstHour);

const waiting = new Promise<never>(() => {});
let release = () => {};
const recorded = new Promise<void>(resolve => { release = resolve; });
const client = {
  available: true,
  product: async (request: { sourceId: string; revision: string; product: string }) => request.product === 'wars'
    ? recorded.then(() => ({ sourceId: request.sourceId, revision: request.revision, product: request.product, data: wars })) : waiting,
  releaseSource: async () => {},
};
const source = { id: 'menu', revision: '1', metadata: { countries: [], branches: [{ id: '1', firstHour: 0, lastHour: 200, gaps: [] }], campaignKey: 'campaign', captureMode: 'full' } };
const track = { id: 'main', roleId: 'main', label: 'main', branchIndex: 0, anchorOffset: null, axisOffset: 0, selectedTag: 'GER', source, branch: { id: '1', firstHour: 0, lastHour: 200, gaps: [] } };
const runtime = { tracks: [track], campaignTracks: [track], defaultRoleId: 'main', clock, client, campaigns: [], recordings: [] } as unknown as WidgetRuntime;

const state: { widget: WidgetSpec; rebuilt: number } = {
  widget: { id: 'war', kind: 'war-contribution', configVersion: 1, title: 'War contribution', campaign: 'default', trackIds: ['unloaded'], countries: { kind: 'tags', tags: ['GER'] },
    time: { mode: 'follow' }, options: { view: 'measure', 'measure@measure': 'casualties' } } as unknown as WidgetSpec,
  rebuilt: 0,
};
const template = { tracks: [track], widgets: [state.widget], layouts: {} } as unknown as DashboardTemplate;
const draft = new OptionDraft(state.widget.options);
const setOption = (key: string, value: OptionValue) => {
  draft.sync(state.widget.options);
  const options = draft.updateAll(widgetOptionFields(state.widget.kind, state.widget.options), { [key]: value });
  if (options) state.widget = { ...state.widget, options };
};
const onApply = (next: WidgetSpec) => { state.rebuilt++; state.widget = next; };

const host = document.body.appendChild(document.createElement('div'));
const root = createRoot(host);
const render = () => root.render(<WidgetCountryControl widget={state.widget} template={template} runtime={runtime} onApply={onApply} setOption={setOption}><span>War</span></WidgetCountryControl>);

const until = (done: () => boolean) => new Promise<void>(resolve => {
  if (done()) return resolve();
  const observer = new MutationObserver(() => {
    if (!done()) return;
    observer.disconnect();
    resolve();
  });
  observer.observe(host.ownerDocument.body, { childList: true, subtree: true, attributes: true, characterData: true });
});
const warRow = () => [...host.ownerDocument.querySelectorAll<HTMLButtonElement>('.war-menu-row')].find(row => row.textContent?.includes('German-Polish War'));

Object.assign(window, { subjectMenu: {
  async chooseWar() {
    render();
    await until(() => !!host.querySelector('.dash-widget-country-button'));
    host.querySelector<HTMLButtonElement>('.dash-widget-country-button')!.click();
    const pending = () => host.ownerDocument.querySelector('.war-menu-pending')?.textContent ?? null;
    await until(() => pending() !== null);
    const loading = { pending: pending(), rows: host.ownerDocument.querySelectorAll('.war-menu-row').length };
    release();
    await until(() => !!warRow());
    const loaded = pending();
    const read = () => {
      const view = resolveWidgetActiveView(state.widget.kind, state.widget.options);
      const options = widgetViewOptions(state.widget.kind, state.widget.options, view);
      return { view, measure: options.measure ?? null, war: options.war ?? null };
    };
    const before = read();
    warRow()!.click();
    return { loading, loaded, rebuilt: state.rebuilt, stored: state.widget.options, before, after: read(), countries: state.widget.countries, title: state.widget.title };
  },
} });
