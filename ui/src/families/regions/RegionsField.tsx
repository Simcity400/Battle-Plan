import { Popover, usePopover } from '../../components/Popover';
import type { DashboardClient } from '../../dashboard/client';
import type { BoundTrack } from '../../dashboard/model';
import type { OptionField, OptionValue } from '../../dashboard/options';
import { SettingLabel } from '../../dashboard/settings/SettingLabel';
import RegionChooser from './RegionChooser.tsx';
import { addRegion, AUTOMATIC_REGIONS, isAutomatic, moveRegion, readRegionChoice, removeRegion, writeRegionChoice } from './regionChoice.ts';
import { useRegionList } from './useRegionList.ts';
import { savedRegionName } from './regionRows.ts';
import { formatScalar } from '../valueFormat.ts';

export default function RegionsField({ field, value, onChange, client, track, hour }: {
  readonly field: Extract<OptionField, { kind: 'regions' }>;
  readonly value: OptionValue;
  readonly onChange: (value: OptionValue) => void;
  readonly client: DashboardClient;
  readonly track: BoundTrack | undefined;
  readonly hour: number | null;
}) {
  const choice = readRegionChoice(value);
  const menu = usePopover('start');
  const list = useRegionList(client, track, true, hour);
  const nameOf = (id: number) => savedRegionName(list, id);
  const inChart = choice.picks.map(id => ({ id, label: nameOf(id) }));
  const hint = isAutomatic(choice) ? field.automatic : choice.hidden.length
    ? `${formatScalar(choice.hidden.length)} removed from the most active`
    : field.hint;
  return <div data-setting={field.key} className="setting-row setting-row-stacked">
    <SettingLabel label={field.label} hint={hint} />
    {choice.picks.length > 0 && <div className="regions-field-list" role="list" aria-label={field.label}>
      {choice.picks.map((id, index) => {
        const name = nameOf(id);
        return <div key={id} className="regions-field-item" role="listitem">
          <span className="regions-field-label">{name}</span>
          <span className="table-settings-column-move">
            <button type="button" aria-label={`Move ${name} up`} disabled={index === 0}
              onClick={() => onChange(writeRegionChoice(moveRegion(choice, id, index - 1)))}>↑</button>
            <button type="button" aria-label={`Move ${name} down`} disabled={index === choice.picks.length - 1}
              onClick={() => onChange(writeRegionChoice(moveRegion(choice, id, index + 1)))}>↓</button>
            <button type="button" aria-label={`Remove ${name}`}
              onClick={() => onChange(writeRegionChoice(removeRegion(choice, id)))}>×</button>
          </span>
        </div>;
      })}
    </div>}
    <div className="regions-field-actions">
      <button type="button" ref={menu.trigger} className="button small quiet" aria-haspopup="dialog" aria-expanded={menu.open}
        onClick={event => menu.openAt(event.currentTarget.getBoundingClientRect())}>Add region</button>
      {!isAutomatic(choice) && <button type="button" className="button small quiet" onClick={() => onChange(writeRegionChoice(AUTOMATIC_REGIONS))}>Reset to automatic</button>}
    </div>
    <Popover menu={menu} className="region-chooser-popover" aria-label="Add region">
      {menu.open && <RegionChooser list={list} inChart={inChart} active={null} seaOnly={field.sea === true} automatic={isAutomatic(choice)}
        onChoose={(id, charted) => { onChange(writeRegionChoice(charted ? removeRegion(choice, id) : addRegion(choice, id))); menu.close(); }}
        onReset={() => { onChange(writeRegionChoice(AUTOMATIC_REGIONS)); menu.close(); }} />}
    </Popover>
  </div>;
}
