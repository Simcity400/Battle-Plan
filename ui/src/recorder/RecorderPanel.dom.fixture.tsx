import { createRoot } from 'react-dom/client';
import { PanelView } from './RecorderPanel';
import type { PanelActions, View } from './RecorderPanel';
import type { RecorderSettings, Snapshot } from './status';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 4; i++) await frame(); }

const settings: RecorderSettings = { recordEveryCampaign: true, autoSave: true, startWithWindows: false, startWithReplay: true, captureMode: 'full', liveCompression: 'low', autoCompress: true,
  campaigns: {}, effectiveSaveFolder: 'C:\Recordings' };
const recording: Snapshot = { phase: 'recording', campaign: '00000000-0000-4000-8000-000000000000', recording: null, name: 'Germany', continued: false, hour: null,
  firstHour: null, savedBytes: null, spoolBytes: null, startedAt: 0, stopping: false, problem: null, automatic: true, session: null, flag: null, captureMode: 'full',
  revision: 1, sessionKey: 1 };
const actions = new Proxy({}, { get: () => () => {} }) as PanelActions;

const host = document.body.appendChild(document.createElement('div'));
const root = createRoot(host);
const seen: number[] = [];
const onPromptSeen = (prompt: number) => { seen.push(prompt); };

export const fixture = {
  async render(view: View, prompt: number, compact = false) {
    root.render(<PanelView snapshot={recording} settings={settings} view={view} asking error={null} now={0} actions={actions} compact={compact}
      shownPrompt={prompt} onPromptSeen={onPromptSeen} />);
    await settle();
  },
  press(selector: string) {
    host.querySelector(selector)?.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true }));
  },
  key(selector: string) {
    host.querySelector(selector)?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Tab', bubbles: true }));
  },
  seen: () => [...seen],
};

(window as unknown as { recorderPanelFixture: typeof fixture }).recorderPanelFixture = fixture;
