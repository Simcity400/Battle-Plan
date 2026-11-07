import { useState } from 'react';
import { Icon } from '../components/Icons';
import { useContextMenu } from '../components/ContextMenu';
import { MAX_PAGE_LABEL, movePage, pageIcon } from '../dashboard/pages';
import type { PageEntry, PageId } from '../dashboard/pages';
import { newId } from './ids.ts';
import type { useSidebarIconMenu } from './appearance/SidebarIconMenu';
import { isSidebarIconId } from './appearance/sidebarIcons';
import { StyledIcon } from './appearance/StyledIcon';
import { commitZone } from './commitCount';

/** Analytics in the sidebar: its pages, added with +, renamed or deleted from a right-click
 * menu (or the context-menu key; F2 renames) and reordered by dragging, or Alt+Up / Alt+Down. */
export default function AnalyticsNav({ pages, page, active, disabled, onChoose, onPages, onDelete, iconMenu }: {
  pages: PageEntry[]; page: PageId; active: boolean; disabled: boolean;
  iconMenu: Pick<ReturnType<typeof useSidebarIconMenu>, 'colour' | 'icon' | 'items'>;
  onChoose: (id: PageId) => void; onPages: (pages: PageEntry[]) => void; onDelete: (id: PageId) => void;
}) {
  const [open, setOpen] = useState(true);
  const [renaming, setRenaming] = useState<PageId | null>(null);
  // `to` is the index the dragged page would land before; `pages.length` is the end.
  const [drag, setDrag] = useState<{ from: number; to: number } | null>(null);
  const contextMenu = useContextMenu('Page actions');
  const add = () => {
    const entry = { id: `page-${newId()}`, label: 'New page' };
    onPages([...pages, entry]);
    onChoose(entry.id);
    setOpen(true);
    setRenaming(entry.id);
  };
  const rename = (id: PageId, label: string) => {
    const name = label.trim().slice(0, MAX_PAGE_LABEL);
    if (name) onPages(pages.map(entry => entry.id === id ? { ...entry, label: name } : entry));
    setRenaming(null);
  };
  const actions = (entry: PageEntry, button: HTMLButtonElement, x: number, y: number) => contextMenu.open(x, y, [
    { label: 'Rename', icon: 'edit', run: () => setRenaming(entry.id) },
    ...(isSidebarIconId(entry.id) ? iconMenu.items(entry.id, button, entry.label) : []),
    { label: 'Delete', icon: 'trash', disabled: pages.length < 2, hint: pages.length < 2 ? 'Keep at least one page' : undefined, run: () => onDelete(entry.id) },
  ]);
  return <>
    <div className="sidebar-group">
      <button id="tab-analytics" type="button" className="sidebar-row sidebar-group-row" data-sidebar-icon="analytics" aria-expanded={open} onClick={() => setOpen(value => !value)}>
        <StyledIcon icon={iconMenu.icon('analytics')} fallback="chart-bars" colour={iconMenu.colour('analytics')} /><span>Analytics</span><Icon name="chevron-down" className="sidebar-group-chevron" /></button>
      <button type="button" className="icon-button small sidebar-group-add" aria-label="Add a page" data-tooltip="Add a page" disabled={disabled} onClick={add}><Icon name="plus" /></button>
    </div>
    {open && <div className="sidebar-page-list">{pages.map((entry, index) => renaming === entry.id
      ? <input key={entry.id} className="sidebar-page-rename" aria-label="Page name" defaultValue={entry.label} maxLength={MAX_PAGE_LABEL} autoFocus
        onFocus={event => event.currentTarget.select()} onBlur={event => rename(entry.id, event.currentTarget.value)}
        onKeyDown={event => { if (event.key === 'Enter') event.currentTarget.blur(); else if (event.key === 'Escape') setRenaming(null); }} />
      : <button key={entry.id} type="button" className="sidebar-row sidebar-subrow" disabled={disabled} draggable={!disabled}
        aria-current={active && page === entry.id ? 'page' : undefined}
        data-drop={drag && drag.to === index ? 'before' : drag && drag.to === pages.length && index === pages.length - 1 ? 'after' : undefined}
        onClick={() => onChoose(entry.id)}
        onContextMenu={event => { event.preventDefault(); actions(entry, event.currentTarget, event.clientX, event.clientY); }}
        onKeyDown={event => {
          if (event.key === 'F2') { event.preventDefault(); setRenaming(entry.id); }
          else if (event.altKey && (event.key === 'ArrowUp' || event.key === 'ArrowDown')) {
            event.preventDefault();
            const to = event.key === 'ArrowUp' ? index - 1 : index + 2;
            if (to >= 0 && to <= pages.length) onPages(movePage(pages, index, to));
          }
          else if (event.key === 'ContextMenu' || (event.key === 'F10' && event.shiftKey)) {
            event.preventDefault();
            const box = event.currentTarget.getBoundingClientRect();
            actions(entry, event.currentTarget, box.left + 24, box.bottom);
          }
        }}
        onDragStart={event => { event.dataTransfer.effectAllowed = 'move'; event.dataTransfer.setData('text/plain', entry.label); setDrag({ from: index, to: index }); }}
        onDragOver={event => {
          if (!drag) return;
          event.preventDefault();
          const box = event.currentTarget.getBoundingClientRect();
          const to = event.clientY < box.top + box.height / 2 ? index : index + 1;
          if (to !== drag.to) setDrag({ ...drag, to });
        }}
        onDrop={event => {
          event.preventDefault();
          if (drag && drag.to !== drag.from && drag.to !== drag.from + 1) onPages(movePage(pages, drag.from, drag.to));
          setDrag(null);
        }}
        onDragEnd={() => setDrag(null)}>
        <StyledIcon icon={isSidebarIconId(entry.id) ? iconMenu.icon(entry.id) : undefined} fallback={pageIcon(entry.id)}
          colour={isSidebarIconId(entry.id) ? iconMenu.colour(entry.id) : undefined} /><span>{entry.label}</span></button>)}</div>}
    {contextMenu.menu}
  </>;
}

commitZone(AnalyticsNav, 'navigation');
