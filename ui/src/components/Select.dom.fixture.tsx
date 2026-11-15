/** Synthetic recorded choices in the production menu and styles. */
import { useState } from 'react';
import { createRoot } from 'react-dom/client';
import { flushSync } from 'react-dom';
import { Select } from './Select';
import type { SelectOption } from './Select';
import '../styles.css';

const longLabel = 'Synthetic recorded choice 100 whose full name is meaningful';
const frame = () => new Promise<void>(resolve => requestAnimationFrame(() => resolve()));
async function settle() { for (let i = 0; i < 8; i++) await frame(); }
const host = document.createElement('div'); document.body.replaceChildren(host);
host.style.cssText = 'width:160px;margin:24px';
document.documentElement.dataset.motion = 'reduced';
const root = createRoot(host);
let updateChoices: (options: SelectOption[]) => void;
let catalogue: SelectOption[] = [], chosen: string | null = null, mounts = 0;
function Choices({ initial }: { initial: SelectOption[] }) {
  const [options, setOptions] = useState(initial);
  updateChoices = next => { catalogue = next; setOptions(next); };
  return <Select label="Recorded choice" value="saved" options={options} onChange={value => { chosen = value; }} />;
}
export const fixture = {
  async mount(loading = false, count = 150) {
    chosen = null;
    catalogue = loading ? [{ value: '', label: 'Automatic' }, { value: 'saved', label: 'Saved choice unavailable' }]
      : this.choices(count);
    root.render(<Choices key={++mounts} initial={catalogue} />); await settle();
  },
  choices(count = 150): SelectOption[] {
    return Array.from({ length: count }, (_, i) => ({ value: i === 100 ? 'saved' : `v${i}`,
      label: i === 100 ? longLabel : `Choice ${i}`, reason: i === 100 ? 'Recorded reason' : undefined }));
  },
  async loadChoices() { updateChoices(this.choices()); await settle(); },
  async replaceChoices(options: SelectOption[]) { updateChoices(options); await settle(); },
  /** The open menu's row tops just after its owner renders again with an equal list, before anything is measured again. */
  sameChoicesNow() {
    const tops = () => [...document.querySelectorAll<HTMLElement>('.select-menu [role="option"]')].map(row => [row.dataset.optionIndex, row.offsetTop]);
    const before = tops();
    flushSync(() => updateChoices(catalogue.map(option => ({ ...option }))));
    return { before, after: tops() };
  },
  async open() { document.querySelector<HTMLButtonElement>('.select-trigger')!.click(); await settle(); },
  async scroll(top: number) { document.querySelector<HTMLElement>('[role="listbox"]')!.scrollTop = top; await settle(); },
  settle,
  snapshot() {
    const menu = document.querySelector<HTMLElement>('.select-menu')!, list = menu.querySelector<HTMLElement>('[role="listbox"]');
    const active = document.activeElement as HTMLElement;
    const rows = [...menu.querySelectorAll<HTMLElement>('[role="option"]')].map(row => ({
      index: Number(row.dataset.optionIndex), label: row.textContent, top: row.offsetTop, height: row.offsetHeight,
      width: row.clientWidth, scrollWidth: row.scrollWidth,
      labelWidth: row.querySelector<HTMLElement>('.menu-item-label')!.clientWidth,
      labelScrollWidth: row.querySelector<HTMLElement>('.menu-item-label')!.scrollWidth,
    }));
    const at = active.dataset.optionIndex;
    return { activeRole: active.getAttribute('role'), activeValue: at === undefined ? null : catalogue[Number(at)]?.value,
      activeLabel: active.textContent, chosen, open: menu.matches(':popover-open'),
      menuWidth: menu.getBoundingClientRect().width, menuLeft: menu.getBoundingClientRect().left,
      menuRight: menu.getBoundingClientRect().right, viewportWidth: document.documentElement.clientWidth,
      listWidth: list?.clientWidth, listHeight: list?.clientHeight, listScrollWidth: list?.scrollWidth, scrollTop: list?.scrollTop, rows };
  },
};
declare global { interface Window { selectFixture: typeof fixture } }
window.selectFixture = fixture;
