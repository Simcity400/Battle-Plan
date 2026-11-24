import { useId, useMemo, useState } from 'react';
import { failureText } from '../../dashboard/queryError.ts';
import type { ActiveRegions } from '../../generated/dashboard';
import { regionChooserGroups, type RegionInChart } from './regionRows.ts';
import type { RegionList } from './useRegionList.ts';
import './regionChooser.css';

export default function RegionChooser({ list, inChart, active, seaOnly, automatic, onChoose, onReset }: {
  readonly list: RegionList;
  readonly inChart: readonly RegionInChart[];
  readonly active: ActiveRegions | null;
  readonly seaOnly: boolean;
  readonly automatic: boolean;
  readonly onChoose: (id: number, inChart: boolean) => void;
  readonly onReset: () => void;
}) {
  const id = useId();
  const [query, setQuery] = useState('');
  const [highlight, setHighlight] = useState(-1);
  const groups = useMemo(() => regionChooserGroups(list, inChart, active, seaOnly, query), [list, inChart, active, seaOnly, query]);
  const flat = useMemo(() => groups.flatMap(group => group.rows), [groups]);
  const optionId = (index: number) => `${id}-region-${index}`;
  const choose = (index: number) => {
    const row = flat[index];
    if (row) onChoose(row.id, row.inChart);
  };
  let index = 0;
  return <div className="region-chooser">
    <input className="region-chooser-search" data-popover-autofocus type="search" placeholder="Search regions" aria-label="Search regions"
      aria-controls={`${id}-list`} aria-activedescendant={highlight >= 0 && highlight < flat.length ? optionId(highlight) : undefined} value={query}
      onChange={event => { setQuery(event.currentTarget.value); setHighlight(-1); }}
      onKeyDown={event => {
        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
          event.preventDefault();
          if (!flat.length) return;
          setHighlight(current => event.key === 'ArrowDown' ? Math.min(flat.length - 1, current + 1) : Math.max(0, current - 1));
        } else if (event.key === 'Enter' && highlight >= 0) {
          event.preventDefault();
          choose(highlight);
        }
      }} />
    <div id={`${id}-list`} className="region-chooser-list" role="listbox" aria-label="Regions">
      {groups.map(group => <div key={group.title} role="group" aria-label={group.title}>
        <div className="region-chooser-group" aria-hidden="true">{group.title}</div>
        {group.note && <p className="region-chooser-note-line">{/[.!?]$/u.test(group.note) ? group.note : `${group.note}.`}</p>}
        {group.rows.map(row => {
          const at = index++;
          return <div key={`${group.title}:${row.id}`} id={optionId(at)} role="option" aria-selected={row.inChart} className="region-chooser-row"
            data-in-chart={row.inChart || undefined} data-active={at === highlight || undefined}
            onPointerEnter={() => setHighlight(at)} onClick={() => choose(at)}>
            <span className="region-chooser-name">{row.label}</span>
            {row.inChart && <span className="region-chooser-note">In chart</span>}
          </div>;
        })}
      </div>)}
      {list.kind === 'loading' && <p className="region-chooser-empty" role="status"><span className="spinner" />Loading regions…</p>}
      {list.kind === 'failed' && <p className="region-chooser-empty" role="alert" data-detail={list.failure.message}>
        {failureText(list.failure, 'Couldn’t load the regions.')} <button type="button" className="button small quiet" onClick={list.retry}>Retry</button></p>}
      {list.kind === 'ready' && !flat.length && <p className="region-chooser-empty">{query ? 'No matching regions' : 'No regions in this recording'}</p>}
    </div>
    <div className="region-chooser-footer">
      <button type="button" className="button small quiet" disabled={automatic} onClick={onReset}>Reset to automatic</button>
    </div>
  </div>;
}
