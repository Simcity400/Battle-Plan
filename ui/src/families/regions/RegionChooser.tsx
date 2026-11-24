import { useId, useMemo, useState } from 'react';
import { failureText } from '../../dashboard/queryError.ts';
import type { RegionActivity } from './regionRows.ts';
import type { RegionList } from './useRegionList.ts';
import './regionChooser.css';

export interface RegionInChart {
  readonly id: number;
  readonly label: string;
  readonly activity?: RegionActivity;
}

interface ChooserRow {
  readonly id: number;
  readonly label: string;
  readonly inChart: boolean;
  readonly activity?: RegionActivity;
}

interface ChooserGroup {
  readonly title: string;
  readonly rows: readonly ChooserRow[];
}

export function regionChooserGroups(list: RegionList, inChart: readonly RegionInChart[], seaOnly: boolean, query: string): readonly ChooserGroup[] {
  const needle = query.trim().toLocaleLowerCase();
  const matches = (label: string) => !needle || label.toLocaleLowerCase().includes(needle);
  const charted = new Set(inChart.map(row => row.id));
  const active = inChart.filter(row => row.activity && matches(row.label)).map(row => ({ id: row.id, label: row.label, inChart: true, activity: row.activity! }));
  const regions = list.kind === 'ready' ? list.regions.filter(region => matches(region.label)) : [];
  const listed = (sea: boolean) => regions.filter(region => region.sea === sea).map(region => ({ id: region.id, label: region.label, inChart: charted.has(region.id) }));
  const groups: ChooserGroup[] = [{ title: 'Active now', rows: active }];
  if (!seaOnly) groups.push({ title: 'Land regions', rows: listed(false) });
  groups.push({ title: 'Sea regions', rows: listed(true) });
  return groups.filter(group => group.rows.length > 0);
}

export default function RegionChooser({ list, inChart, seaOnly, automatic, onChoose, onReset }: {
  readonly list: RegionList;
  readonly inChart: readonly RegionInChart[];
  readonly seaOnly: boolean;
  readonly automatic: boolean;
  readonly onChoose: (id: number, inChart: boolean) => void;
  readonly onReset: () => void;
}) {
  const id = useId();
  const [query, setQuery] = useState('');
  const [active, setActive] = useState(-1);
  const groups = useMemo(() => regionChooserGroups(list, inChart, seaOnly, query), [list, inChart, seaOnly, query]);
  const flat = useMemo(() => groups.flatMap(group => group.rows), [groups]);
  const optionId = (index: number) => `${id}-region-${index}`;
  const choose = (index: number) => {
    const row = flat[index];
    if (row) onChoose(row.id, row.inChart);
  };
  let index = 0;
  return <div className="region-chooser">
    <input className="region-chooser-search" data-popover-autofocus type="search" placeholder="Search regions" aria-label="Search regions"
      aria-controls={`${id}-list`} aria-activedescendant={active >= 0 && active < flat.length ? optionId(active) : undefined} value={query}
      onChange={event => { setQuery(event.currentTarget.value); setActive(-1); }}
      onKeyDown={event => {
        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
          event.preventDefault();
          if (!flat.length) return;
          setActive(current => event.key === 'ArrowDown' ? Math.min(flat.length - 1, current + 1) : Math.max(0, current - 1));
        } else if (event.key === 'Enter' && active >= 0) {
          event.preventDefault();
          choose(active);
        }
      }} />
    <div id={`${id}-list`} className="region-chooser-list" role="listbox" aria-label="Regions">
      {groups.map(group => <div key={group.title} role="group" aria-label={group.title}>
        <div className="region-chooser-group" aria-hidden="true">{group.title}</div>
        {group.rows.map(row => {
          const at = index++;
          return <div key={`${group.title}:${row.id}`} id={optionId(at)} role="option" aria-selected={row.inChart} className="region-chooser-row"
            data-in-chart={row.inChart || undefined} data-active={at === active || undefined}
            onPointerEnter={() => setActive(at)} onClick={() => choose(at)}>
            <span className="region-chooser-name">{row.label}</span>
            {row.activity
              ? <span className="region-chooser-note">{row.activity.color && <i className="region-chooser-swatch" style={{ background: row.activity.color }} />}{row.activity.text}</span>
              : row.inChart && <span className="region-chooser-note">In chart</span>}
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
