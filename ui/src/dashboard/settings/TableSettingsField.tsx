import { Segmented } from '../../components/Segmented';
import { SettingLabel } from './SettingLabel';
import type { OptionValue } from '../options';
import { BARS_BEHIND_CHOICES, ROW_HEIGHT_CHOICES, type BarsBehindMode } from '../../families/settingsControls.ts';
import {
  encodeTableSettings,
  tableSettingsOf,
  updateTableSettings,
  type TableBase,
  type TableColumnDefaults,
  type TableSettings,
} from '../../families/table/settings.ts';

export function TableSettingsField({ field, value, onChange }: {
  field: { key: string; label: string; hint?: string; columns: readonly TableColumnDefaults[]; base: TableBase };
  value: OptionValue;
  onChange: (value: OptionValue) => void;
}) {
  const settings = tableSettingsOf(value, field.columns, field.base);
  const save = (update: (next: TableSettings) => void) => onChange(encodeTableSettings(updateTableSettings(settings, update)));
  return <div data-setting={field.key} className="table-settings-field" role="group" aria-label={field.label}>
    <div className="setting-row setting-row-stacked">
      <SettingLabel label="Rows" />
      <Segmented label="Rows" value={settings.density} options={ROW_HEIGHT_CHOICES}
        onChange={value => save(next => { next.density = value as TableSettings['density']; })} />
    </div>
    <div className="setting-row setting-row-stacked">
      <SettingLabel label="Bars behind figures" />
      <Segmented label="Bars behind figures" value={settings.barsBehind ?? 'sorted'} options={BARS_BEHIND_CHOICES}
        onChange={value => save(next => { next.barsBehind = value as BarsBehindMode; })} />
    </div>
    <div className="setting-row setting-row-stacked">
      <SettingLabel label="When columns don't fit" />
      <div className="table-settings-radio" role="radiogroup" aria-label="When columns don't fit">
        <label><input type="radio" name={`${field.key}-overflow`} value="tooltip" checked={settings.overflow === 'tooltip'}
          onChange={() => save(next => { next.overflow = 'tooltip'; })} />Hide them into the tooltip</label>
        <label><input type="radio" name={`${field.key}-overflow`} value="scroll" checked={settings.overflow === 'scroll'}
          onChange={() => save(next => { next.overflow = 'scroll'; })} />Scroll, first column fixed</label>
      </div>
    </div>
    {field.columns.length > 0 && <div className="setting-row setting-row-stacked" role="group" aria-label="Columns">
      <SettingLabel label="Columns" />
      <div className="table-settings-columns">
        {settings.order.map((key, index) => {
          const column = field.columns.find(item => item.key === key);
          if (!column) return null;
          return <div key={column.key} className="table-settings-column">
            <label><input type="checkbox" checked={settings.columns[column.key]?.show ?? false}
              onChange={event => save(next => { if (next.columns[column.key]) next.columns[column.key]!.show = event.currentTarget.checked; })} />{column.label}</label>
            <span className="table-settings-column-move">
              <button type="button" aria-label={`Move ${column.label} left`} disabled={index === 0}
                onClick={() => save(next => { [next.order[index - 1], next.order[index]] = [next.order[index]!, next.order[index - 1]!]; })}>↑</button>
              <button type="button" aria-label={`Move ${column.label} right`} disabled={index === settings.order.length - 1}
                onClick={() => save(next => { [next.order[index], next.order[index + 1]] = [next.order[index + 1]!, next.order[index]!]; })}>↓</button>
            </span>
          </div>;
        })}
      </div>
    </div>}
  </div>;
}
