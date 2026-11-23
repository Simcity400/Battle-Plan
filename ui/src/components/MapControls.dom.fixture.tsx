import '../styles.css';

const asked: { view: string | null; menuOpen: boolean }[] = [];
let answer: { error?: string; warning?: string } = {};
Object.assign(globalThis, { isTauri: true });
Object.assign(window, { __TAURI_INTERNALS__: { invoke: async (command: string, args: { viewId?: string | null }) => {
  if (command !== 'export_map') throw new Error(`unexpected command ${command}`);
  asked.push({ view: args.viewId ?? null, menuOpen: panel()?.matches(':popover-open') ?? false });
  if (answer.error) throw answer.error;
  return { message: 'Saved map.png: branch 1, hour 12', warning: answer.warning ?? null };
} } });

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle(frames = 6) { for (let at = 0; at < frames; at++) await frame(); }
const panel = () => document.querySelector<HTMLElement>('.map-controls-menu');
const button = (label: string) => [...document.querySelectorAll<HTMLButtonElement>('button')].find(item => item.textContent?.includes(label));

const [{ createElement }, { createRoot }, { default: MapControls }] = await Promise.all([import('react'), import('react-dom/client'), import('./MapControls')]);
createRoot(document.body.appendChild(document.createElement('div'))).render(createElement(MapControls, {
  source: null, client: {} as never, metadata: { branches: [] } as never, branchId: null, onBranch() {},
  options: { country: null, fog: true } as never, onOptions() {}, viewId: 'widget-1',
}));

export const fixture = {
  async ready() { await settle(); return !!document.querySelector('.map-controls-trigger'); },
  async exportWith(error: string | null, warning?: string) {
    answer = error ? { error } : { warning };
    document.querySelector<HTMLButtonElement>('.map-controls-trigger')!.click();
    await settle();
    const opened = panel()!.matches(':popover-open');
    button('Export map image')!.click();
    await settle(12);
    const shown = panel()!.matches(':popover-open');
    const said = panel()!.querySelector('.map-menu-error > span')?.textContent ?? null;
    const retry = panel()!.querySelector('.map-menu-error button')?.textContent ?? null;
    const note = panel()!.querySelector('.map-menu-note[role="status"]')?.textContent ?? null;
    if (shown) panel()!.hidePopover();
    await settle();
    return { opened, asked: asked.splice(0), shown, said, retry, note };
  },
};
Object.assign(window, { mapControls: fixture });
