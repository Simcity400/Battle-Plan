import { useRef, useState } from 'react';
import { Segmented } from '../../components/Segmented';
import { SettingLabel } from './SettingLabel';
import type { OptionValue } from '../options';
import {
  encodeTableSettings,
  isCountryColumn,
  tableDefaults,
  tableSettingsOf,
  updateTableSettings,
  type TableBarsMode,
  type TableBase,
  type TableColumnDefaults,
  type TableDensity,
  type TableSettings,
} from '../../families/table/settings.ts';

const GRIP = <svg viewBox="0 0 12 16" aria-hidden="true" width="10" height="14">
  <circle cx="3.5" cy="4" r="1.15" fill="currentColor" />
  <circle cx="8.5" cy="4" r="1.15" fill="currentColor" />
  <circle cx="3.5" cy="8" r="1.15" fill="currentColor" />
  <circle cx="8.5" cy="8" r="1.15" fill="currentColor" />
  <circle cx="3.5" cy="12" r="1.15" fill="currentColor" />
  <circle cx="8.5" cy="12" r="1.15" fill="currentColor" />
</svg>;

const EYE_ON = <svg viewBox="0 0 16 16" aria-hidden="true" width="14" height="14">
  <path d="M1.8 8s2.3-4.2 6.2-4.2S14.2 8 14.2 8s-2.3 4.2-6.2 4.2S1.8 8 1.8 8Z" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
  <circle cx="8" cy="8" r="1.8" fill="none" stroke="currentColor" strokeWidth="1.4" />
</svg>;

const EYE_OFF = <svg viewBox="0 0 16 16" aria-hidden="true" width="14" height="14">
  <path d="M2 8s2.2-4 6-4 6 4 6 4-2.2 4-6 4-6-4-6-4Z" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinejoin="round" />
  <path d="M3 13 13 3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" />
</svg>;

export function TableSettingsField({ field, value, onChange }: {
  field: { key: string; label: string; hint?: string; columns: readonly TableColumnDefaults[]; base: TableBase };
  value: OptionValue;
  onChange: (value: OptionValue) => void;
}) {
  const settings = tableSettingsOf(value, field.columns, field.base);
  const save = (update: (next: TableSettings) => void) => onChange(encodeTableSettings(updateTableSettings(settings, update)));
  const dragKeyRef = useRef<string | null>(null);
  const [overKey, setOverKey] = useState<string | null>(null);

  const moveColumn = (fromKey: string, toKey: string) => {
    if (fromKey === toKey) return;
    save(next => {
      const fromIndex = next.order.indexOf(fromKey);
      const toIndex = next.order.indexOf(toKey);
      if (fromIndex < 0 || toIndex < 0) return;
      const [moved] = next.order.splice(fromIndex, 1);
      next.order.splice(toIndex, 0, moved!);
    });
  };

  const hasCustomWidths = Object.values(settings.columns).some(col => col.width !== null);
  const defaults = tableDefaults(field.columns, field.base);
  const defaultOrder = defaults.order;
  const orderChanged = settings.order.length !== defaultOrder.length
    || settings.order.some((key, index) => defaultOrder[index] !== key)
    || field.columns.some(col => {
      const cur = settings.columns[col.key];
      const def = defaults.columns[col.key];
      return cur && def && (cur.show !== def.show || cur.showNames !== def.showNames);
    });

  return <div data-setting={field.key} className="table-settings-field" role="group" aria-label={field.label}>
    <div className="setting-row setting-row-stacked">
      <SettingLabel label="Bars behind figures" />
      <Segmented
        label="Bars behind figures"
        value={settings.bars}
        options={[
          { value: 'all', label: 'All' },
          { value: 'sorted', label: 'Sorted' },
          { value: 'off', label: 'Off' },
        ]}
        onChange={next => save(draft => { draft.bars = next as TableBarsMode; })}
      />
    </div>
    <div className="setting-row setting-row-stacked">
      <SettingLabel label="Row height" />
      <Segmented
        label="Row height"
        value={settings.density}
        options={[
          { value: 'compact', label: 'Compact' },
          { value: 'normal', label: 'Normal' },
          { value: 'comfortable', label: 'Comfortable' },
        ]}
        onChange={next => save(draft => { draft.density = next as TableDensity; })}
      />
    </div>
    <div className="setting-row table-settings-actions-row">
      <button
        type="button"
        className="table-settings-link"
        onClick={() => save(draft => {
          for (const key of Object.keys(draft.columns)) draft.columns[key]!.width = null;
        })}
      >
        Fit all columns
      </button>
      {hasCustomWidths && <button
        type="button"
        className="table-settings-link"
        onClick={() => save(draft => {
          for (const key of Object.keys(draft.columns)) draft.columns[key]!.width = null;
        })}
      >
        Reset widths
      </button>}
    </div>
    {field.columns.length > 0 && <div className="setting-row setting-row-stacked" role="group" aria-label="Columns">
      <div className="table-settings-columns-head">
        <SettingLabel label="Columns" />
        {orderChanged && <button
          type="button"
          className="table-settings-link"
          onClick={() => save(draft => {
            draft.order = [...defaultOrder];
            for (const col of field.columns) {
              const def = defaults.columns[col.key];
              if (def && draft.columns[col.key]) {
                draft.columns[col.key]!.show = def.show;
                draft.columns[col.key]!.as = def.as;
                if (def.showNames !== undefined) draft.columns[col.key]!.showNames = def.showNames;
              }
            }
          })}
        >
          Reset to default order
        </button>}
      </div>
      <div className="table-settings-columns">
        <div className="table-settings-column table-settings-rank-row">
          <span className="table-settings-grip is-static" aria-hidden="true">#</span>
          <span className="table-settings-col-name">Rank</span>
          <button
            type="button"
            className={`table-settings-eye${settings.showRank ? '' : ' is-hidden'}`}
            aria-label={settings.showRank ? 'Hide Rank column' : 'Show Rank column'}
            aria-pressed={settings.showRank}
            onClick={() => save(draft => { draft.showRank = !draft.showRank; })}
          >
            {settings.showRank ? EYE_ON : EYE_OFF}
          </button>
        </div>
        {settings.order.map((key, index) => {
          const column = field.columns.find(item => item.key === key);
          if (!column) return null;
          const own = settings.columns[column.key];
          const shown = own?.show ?? true;
          const country = isCountryColumn(column);
          const showNames = own?.showNames ?? (own?.as === 'flagName');
          const locked = column.hideable === false || index === 0;
          return <div
            key={column.key}
            className={`table-settings-column${shown ? '' : ' is-hidden'}${overKey === column.key ? ' is-drop-target' : ''}`}
            draggable
            onDragStart={event => {
              dragKeyRef.current = column.key;
              event.dataTransfer.effectAllowed = 'move';
              event.dataTransfer.setData('text/plain', column.key);
            }}
            onDragOver={event => {
              event.preventDefault();
              if (dragKeyRef.current && dragKeyRef.current !== column.key) setOverKey(column.key);
            }}
            onDragLeave={() => { if (overKey === column.key) setOverKey(null); }}
            onDrop={event => {
              event.preventDefault();
              const from = dragKeyRef.current;
              dragKeyRef.current = null;
              setOverKey(null);
              if (from) moveColumn(from, column.key);
            }}
            onDragEnd={() => {
              dragKeyRef.current = null;
              setOverKey(null);
            }}
          >
            <span className="table-settings-grip" aria-hidden="true" title="Drag to reorder">{GRIP}</span>
            <span className="table-settings-col-name">{column.label}</span>
            {country && <button
              type="button"
              className={`table-settings-names-toggle${showNames ? ' is-on' : ''}`}
              role="switch"
              aria-checked={showNames}
              aria-label={`Show names for ${column.label}`}
              onClick={() => save(draft => {
                const entry = draft.columns[column.key];
                if (!entry) return;
                const next = !showNames;
                entry.showNames = next;
                entry.as = next ? 'flagName' : 'flag';
              })}
            >
              Names
            </button>}
            <button
              type="button"
              className={`table-settings-eye${shown ? '' : ' is-hidden'}`}
              disabled={locked}
              aria-label={shown ? `Hide ${column.label}` : `Show ${column.label}`}
              aria-pressed={shown}
              onClick={() => save(draft => {
                if (draft.columns[column.key]) draft.columns[column.key]!.show = !shown;
              })}
            >
              {shown ? EYE_ON : EYE_OFF}
            </button>
          </div>;
        })}
      </div>
    </div>}
  </div>;
}
