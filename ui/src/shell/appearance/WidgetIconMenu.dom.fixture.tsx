import { useContext } from 'react';
import { createRoot } from 'react-dom/client';
import { rasteriseIconImage } from './iconImage';
import { useAppearance } from './useAppearance';
import { WidgetIconsContext, useWidgetIconMenu } from './WidgetIconMenu';
import '../../styles.css';

const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 8; i++) await frame(); }
const host = document.createElement('div'); document.body.replaceChildren(host);
document.documentElement.dataset.motion = 'reduced';
localStorage.clear();
const root = createRoot(host);

function Header() {
  const icons = useContext(WidgetIconsContext);
  return <article className="dash-widget" style={{ margin: 40, width: 320 }}>
    <header className="dash-widget-header">
      <span className="dash-widget-icon" data-widget-icon="w1" onContextMenu={event => {
        event.preventDefault(); icons.openMenu('navy-size', event.currentTarget, 'Navy size', event.clientX, event.clientY);
      }}>icon</span>
      <h2>Navy size</h2>
      <button type="button" data-widget-settings aria-label="Configure Navy size">gear</button>
    </header>
  </article>;
}
function App() {
  const appearance = useAppearance();
  const { icons, menu } = useWidgetIconMenu(appearance);
  return <WidgetIconsContext.Provider value={icons}><Header /><button type="button" id="elsewhere">Elsewhere</button>{menu}</WidgetIconsContext.Provider>;
}

export const fixture = {
  async mount() { root.render(<App />); await settle(); },
  async rightClickIcon() {
    const icon = document.querySelector<HTMLElement>('[data-widget-icon]')!, box = icon.getBoundingClientRect();
    icon.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: box.left + 4, clientY: box.top + 4 }));
    await settle();
  },
  async choose(label: string) {
    const item = [...document.querySelectorAll<HTMLButtonElement>('.context-menu .menu-item')].find(button => button.textContent?.startsWith(label));
    item!.click(); await settle();
  },
  async focusElsewhere() { document.getElementById('elsewhere')!.focus(); await settle(); },
  async rasteriseUploads() {
    const canvas = document.createElement('canvas'); canvas.width = 20; canvas.height = 10;
    const paint = canvas.getContext('2d')!; paint.fillStyle = '#c33'; paint.fillRect(0, 0, 20, 10);
    const png = await new Promise<Blob>(resolve => canvas.toBlob(blob => resolve(blob!), 'image/png'));
    const svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="6" fill="#3c3"/></svg>';
    const results = await Promise.all([
      rasteriseIconImage(new File([png], 'flag.png', { type: 'image/png' })),
      rasteriseIconImage(new File([svg], 'badge.svg', { type: '' })),
    ]);
    return results.map(result => typeof result === 'string' && result.startsWith('data:image/png;base64,'));
  },
  settle,
  snapshot() {
    const active = document.activeElement as HTMLElement | null;
    const picker = document.querySelector<HTMLElement>('.icon-style-picker');
    return {
      active: active === document.body ? 'body' : active?.matches('[data-widget-settings]') ? 'settings' : active?.id || active?.getAttribute('aria-label') || active?.tagName || null,
      pickerOpen: !!picker?.matches(':popover-open'),
      focusInPicker: !!(active && picker?.contains(active)),
    };
  },
};
declare global { interface Window { iconFixture: typeof fixture } }
window.iconFixture = fixture;
