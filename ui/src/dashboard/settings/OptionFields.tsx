import { useEffect, useState } from 'react';
import { Select } from '../../components/Select';
import { SettingLabel } from './SettingLabel';
import type { ReactNode } from 'react';
import { ChipSet } from '../../components/ChipSet';
import ContentSetPicker from '../../components/ContentSetPicker';
import { Segmented } from '../../components/Segmented';
import { contentSet, storableToken, toggleContentSet, UNSTORABLE } from '../options';
import type { DashboardClient } from '../client';
import type { BoundTrack } from '../model';
import RegionsField from '../../families/regions/RegionsField.tsx';
import type { OptionField, OptionValue } from '../options';
import { listedChoices, unavailableChoices } from '../definitions';
import type { OptionChoices } from '../definitions';
import { contentTitleHint } from '../contentTitles';
import { leaveNumber, numberText, receiveNumber, typeNumber } from './numberDraft';
import type { NumberDraft, NumberStep } from './numberDraft';
import { TableSettingsField } from './TableSettingsField.tsx';

const SEGMENTED_MAX = 4;

export interface RegionSource { readonly client: DashboardClient; readonly track: BoundTrack | undefined; readonly hour: number | null }

export function OptionFields({ fields, values, choices, regionSource, onChange }: {
  fields: readonly OptionField[]; values: Readonly<Record<string, OptionValue>>; choices?: OptionChoices; regionSource?: RegionSource;
  onChange: (key: string, value: OptionValue) => void;
}) {
  return <>{fields.map(field => <OptionRow key={field.key} field={field} value={values[field.key]} choices={choices?.[field.key]}
    regionSource={regionSource} onChange={value => onChange(field.key, value)} />)}</>;
}

function OptionRow({ field, value, choices, regionSource, onChange }: {
  field: OptionField; value: OptionValue; choices?: OptionChoices[string]; regionSource?: RegionSource; onChange: (value: OptionValue) => void;
}) {
  const caption = <SettingLabel label={field.label} hint={field.hint} />;
  switch (field.kind) {
    case 'toggle':
      return <label data-setting={field.key} className="setting-row">{caption}
        <input type="checkbox" role="switch" checked={value === true} onChange={event => onChange(event.currentTarget.checked)} /></label>;
    case 'choice':
      return field.choices.length <= SEGMENTED_MAX
        ? <div data-setting={field.key} className="setting-row setting-row-stacked">{caption}
          <Segmented label={field.label} value={String(value)} options={field.choices} onChange={onChange} /></div>
        : <div data-setting={field.key} className="setting-row">{caption}<Select label={field.label} value={String(value)} options={field.choices} onChange={onChange} /></div>;
    case 'content-choice': {
      const listed = listedChoices(choices) ?? [];
      if (field.catalogue) {
        const chosen = typeof value === 'string' && value ? value : '';
        const unavailable = unavailableChoices(choices);
        const missing = !unavailable ? 'Saved choice unavailable' : 'loading' in unavailable ? 'Loading…' : 'Choices unavailable';
        const shown = chosen ? listed.find(choice => choice.value === chosen)?.label ?? missing : field.automatic;
        return <div data-setting={field.key} className="setting-row setting-row-stacked">{caption}
          <ContentSetPicker single label={field.label} automatic={field.automatic} selected={chosen ? [chosen] : []} trigger={<span>{shown}</span>}
            choices={listed.map(({ detail, ...choice }) => detail ? { ...choice, hint: detail } : choice)} unavailable={unavailable}
            onReset={() => onChange('')} onToggle={token => { if (token !== chosen) onChange(token); }} /></div>;
      }
      const kept = typeof value === 'string' && value && !listed.some(choice => choice.value === value)
        ? [{ value, label: 'Saved choice unavailable', reason: contentTitleHint(value, 'Not in this campaign.') }] : [];
      return <div data-setting={field.key} className="setting-row">{caption}
        <Select label={field.label} value={String(value)} options={[{ value: '', label: field.automatic }, ...listed, ...kept]} onChange={onChange} /></div>;
    }
    case 'content-set': {
      const chosen = contentSet(value);
      if (field.catalogue) return <div data-setting={field.key} className="setting-row setting-row-stacked">{caption}
        <ContentSetPicker label={field.label} automatic={field.automatic} choices={listedChoices(choices) ?? []} selected={chosen}
          onReset={() => onChange('')} onToggle={token => { const next = toggleContentSet(value, token, Number.MAX_SAFE_INTEGER); if (next !== null) onChange(next); }} />
      </div>;
      const listed = listedChoices(choices) ?? [];
      const kept = chosen.filter(token => !listed.some(choice => choice.value === token)).map(token => ({ value: token, label: 'Saved choice unavailable', hint: contentTitleHint(token, 'Not in this campaign.') }));
      const chips = [...listed, ...kept].map(chip => ({ ...chip, unavailable: storableToken(chip.value) ? undefined : UNSTORABLE }));
      return <div data-setting={field.key} className="setting-row setting-row-stacked">
        <SettingLabel label={field.label} hint={chosen.length ? field.hint : `${field.automatic}${field.hint ? ` · ${field.hint}` : ''}`} />
        {chips.length ? <ChipSet label={field.label} chips={chips} selected={chosen} max={Number.MAX_SAFE_INTEGER}
          onToggle={token => { const next = toggleContentSet(value, token, Number.MAX_SAFE_INTEGER); if (next != null) onChange(next); }} />
          : <p className="field-hint">Nothing to choose from yet.</p>}
      </div>;
    }
    case 'regions':
      return regionSource ? <RegionsField field={field} value={value} onChange={onChange} client={regionSource.client} track={regionSource.track} hour={regionSource.hour} /> : null;
    case 'number':
      return <NumberRow field={field} value={typeof value === 'number' ? value : null} caption={caption} onChange={onChange} />;
    case 'table':
      return <TableSettingsField field={field} value={value} onChange={onChange} />;
  }
}

function NumberRow({ field, value, caption, onChange }: {
  field: Extract<OptionField, { kind: 'number' }>; value: number | null; caption: ReactNode; onChange: (value: OptionValue) => void;
}) {
  const [draft, setDraft] = useState<NumberDraft>({ text: numberText(value) });
  useEffect(() => setDraft(current => receiveNumber(current, value)), [value]);
  const apply = (step: NumberStep) => { setDraft(step.draft); if (step.save !== undefined) onChange(step.save); };
  return <label data-setting={field.key} className="setting-row">{caption}
    <span className="option-number"><input type="number" min={field.min} max={field.max} step={field.step} value={draft.text}
      placeholder={field.default === null ? 'Auto' : undefined} onChange={event => apply(typeNumber(field, draft, event.currentTarget.value, value))}
      onBlur={() => apply(leaveNumber(field, draft, value))} onKeyDown={event => { if (event.key === 'Enter') event.currentTarget.blur(); }} />{field.unit && <small>{field.unit}</small>}</span></label>;
}
