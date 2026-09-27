import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { Segmented } from '../../components/Segmented';
import { validOption } from '../options';
import type { OptionField, OptionValue } from '../options';
import type { OptionChoices } from '../definitions';

/** Short choice lists are a segmented row; longer ones a drop-down. */
const SEGMENTED_MAX = 4;

/** Renders a widget's declared option fields; each change saves at once. */
export function OptionFields({ fields, values, choices, onChange }: {
  fields: readonly OptionField[]; values: Readonly<Record<string, OptionValue>>; choices?: OptionChoices;
  onChange: (key: string, value: OptionValue) => void;
}) {
  return <>{fields.map(field => <OptionRow key={field.key} field={field} value={values[field.key]} choices={choices?.[field.key]}
    onChange={value => onChange(field.key, value)} />)}</>;
}

function OptionRow({ field, value, choices, onChange }: {
  field: OptionField; value: OptionValue; choices?: readonly { value: string; label: string }[]; onChange: (value: OptionValue) => void;
}) {
  const caption = <span>{field.label}{field.hint && <small>{field.hint}</small>}</span>;
  switch (field.kind) {
    case 'toggle':
      return <label className="setting-row">{caption}
        <input type="checkbox" role="switch" checked={value === true} onChange={event => onChange(event.currentTarget.checked)} /></label>;
    case 'choice':
      return field.choices.length <= SEGMENTED_MAX
        ? <div className="setting-row setting-row-stacked">{caption}
          <Segmented label={field.label} value={String(value)} options={field.choices} onChange={onChange} /></div>
        : <label className="setting-row">{caption}
          <select value={String(value)} onChange={event => onChange(event.currentTarget.value)}>
            {field.choices.map(choice => <option key={choice.value} value={choice.value}>{choice.label}</option>)}
          </select></label>;
    case 'content-choice': {
      // A saved choice this campaign lacks stays selectable, so opening the panel never changes it.
      const listed = choices ?? [];
      const kept = typeof value === 'string' && value && !listed.some(choice => choice.value === value)
        ? [{ value, label: `${value} (not in this campaign)` }] : [];
      return <label className="setting-row">{caption}
        <select value={String(value)} onChange={event => onChange(event.currentTarget.value)}>
          <option value="">{field.automatic}</option>
          {[...listed, ...kept].map(choice => <option key={choice.value} value={choice.value}>{choice.label}</option>)}
        </select></label>;
    }
    case 'number':
      return <NumberRow field={field} value={typeof value === 'number' ? value : null} caption={caption} onChange={onChange} />;
  }
}

/** Keeps a typed number local and saves it on blur or Enter when it is legal; an
 * empty box means automatic when the field allows it. */
function NumberRow({ field, value, caption, onChange }: {
  field: Extract<OptionField, { kind: 'number' }>; value: number | null; caption: ReactNode; onChange: (value: OptionValue) => void;
}) {
  const [text, setText] = useState(value == null ? '' : String(value));
  useEffect(() => setText(value == null ? '' : String(value)), [value]);
  const commit = () => {
    const raw = text.trim();
    if (raw === '') { if (field.default === null && value !== null) onChange(null); else setText(value == null ? '' : String(value)); return; }
    const number = Number(raw);
    if (validOption(field, number)) { if (number !== value) onChange(number); } else setText(value == null ? '' : String(value));
  };
  return <label className="setting-row">{caption}
    <span className="option-number"><input type="number" min={field.min} max={field.max} step={field.step} value={text}
      placeholder={field.default === null ? 'Auto' : undefined} onChange={event => setText(event.currentTarget.value)}
      onBlur={commit} onKeyDown={event => { if (event.key === 'Enter') event.currentTarget.blur(); }} />{field.unit && <small>{field.unit}</small>}</span></label>;
}
