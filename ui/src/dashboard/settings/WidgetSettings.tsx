import { useEffect, useState } from 'react';
import { SettingLabel } from './SettingLabel';
import type { ReactNode } from 'react';
import { Icon } from '../../components/Icons';
import CampaignPicker from '../CampaignPicker';
import { resolveWidgetCampaign } from '../campaigns';
import { widgetDefinition } from '../definitions';
import type { OptionChoices, WidgetDefinition, WidgetRuntime } from '../definitions';
import { WIDGET_CONTENT_SCALES } from '../model';
import type { DashboardTemplate, WidgetSpec } from '../model';
import { resolveOptions, sectionFields, setOption } from '../options';
import type { OptionSection, OptionValue } from '../options';
import { SECTION_TITLES, settingsSections } from '../settingsSections';
import type { SettingsSection } from '../settingsSections';
import { CountriesSection } from './CountriesSection';
import { OptionFields } from './OptionFields';
import { TimeSection } from './TimeSection';
import { FactionColoursEditor } from '../FactionColours';
import './settings.css';

/** Every widget's settings, in the same sections and order for all widgets. Each
 * change is a view preference and saves at once (`onChange(widget, true)`). */
export function WidgetSettings(props: {
  widget: WidgetSpec; template: DashboardTemplate; runtime: WidgetRuntime;
  onChange: (widget: WidgetSpec, view?: boolean) => void;
}) {
  const use = widgetDefinition(props.widget.kind)?.useOptionChoices;
  // Content choices come from a hook; it runs in its own component so the panel's hooks never change.
  // It sees the widget with its campaign resolved, as the widget itself does.
  return use ? <WithChoices key={props.widget.kind} use={use} widget={resolveWidgetCampaign(props.widget, props.template, props.runtime.defaultRoleId)} runtime={props.runtime}>
    {choices => <SettingsBody {...props} choices={choices} />}</WithChoices> : <SettingsBody {...props} />;
}

function WithChoices({ use, widget, runtime, children }: {
  use: NonNullable<WidgetDefinition['useOptionChoices']>; widget: WidgetSpec; runtime: WidgetRuntime; children: (choices: OptionChoices) => ReactNode;
}) {
  return <>{children(use(widget, runtime))}</>;
}

function SettingsBody({ widget, template, runtime, onChange, choices }: {
  widget: WidgetSpec; template: DashboardTemplate; runtime: WidgetRuntime;
  onChange: (widget: WidgetSpec, view?: boolean) => void; choices?: OptionChoices;
}) {
  const definition = widgetDefinition(widget.kind);
  const values = resolveOptions(definition?.options, widget.options);
  const effective = resolveWidgetCampaign(widget, template, runtime.defaultRoleId);
  const reference = runtime.tracks.find(track => track.id === effective.trackIds[0]);
  const save = (next: WidgetSpec) => onChange(next, true);
  const setValue = (key: string, value: OptionValue) => {
    const options = setOption(definition?.options, widget.options, key, value);
    if (!options) return;
    // A title that still names the previous content choice follows the new one.
    const label = (choice: OptionValue) => choices?.[key]?.find(item => item.value === choice)?.label;
    const field = definition?.options?.find(item => item.key === key);
    const named = field?.kind === 'content-choice' && field.titleChoice !== false && [definition?.title, label(values[key])].includes(widget.title);
    const title = named ? label(value) ?? definition?.title ?? widget.title : widget.title;
    save({ ...widget, options, title });
  };
  const fields = (section: OptionSection) => <OptionFields fields={sectionFields(definition?.options, section, values, widget.countries, widget.time.mode)} values={values}
    choices={choices} onChange={setValue} />;
  const body = (section: SettingsSection) => {
    switch (section) {
      case 'campaign': return <CampaignPicker widget={widget} template={template} runtime={runtime} />;
      case 'countries': return <><CountriesSection widget={widget} reference={reference} runtime={runtime} limits={definition?.countries ?? { max: 1 }}
        onChange={countries => save({ ...widget, countries })} />{fields('countries')}</>;
      case 'time': return <>{definition?.data && <TimeSection widget={widget} effective={effective} reference={reference} runtime={runtime}
        modes={definition.timeModes} onChange={time => save({ ...widget, time })} />}{fields('time')}</>;
      case 'size': return <SizeStepper widget={widget} onChange={save} />;
      case 'factionColours': return reference ? <FactionColoursEditor key={`${reference.source.id}:${reference.source.revision}`} client={runtime.client} source={reference.source} />
        : <p className="field-hint">Load a campaign to edit its faction colours.</p>;
      case 'options': case 'axes': case 'interaction': return fields(section);
    }
  };
  return <div className="widget-settings-content">
    <TitleField widget={widget} onChange={save} />
    {settingsSections(definition, values, widget.countries, widget.time.mode).map(section => <section key={section} className="settings-section" aria-label={SECTION_TITLES[section]}>
      <h3>{SECTION_TITLES[section]}</h3>{body(section)}
    </section>)}
  </div>;
}

/** Keeps the title local while typing and saves it on blur or Enter; an empty title
 * goes back to the saved one. */
function TitleField({ widget, onChange }: { widget: WidgetSpec; onChange: (widget: WidgetSpec) => void }) {
  const [title, setTitle] = useState(widget.title);
  useEffect(() => setTitle(widget.title), [widget.title]);
  const commit = () => {
    const value = title.trim();
    if (value && value !== widget.title) onChange({ ...widget, title: value }); else setTitle(widget.title);
  };
  return <label className="field"><span>Title</span><input value={title} onChange={event => setTitle(event.currentTarget.value)}
    onBlur={commit} onKeyDown={event => { if (event.key === 'Enter') event.currentTarget.blur(); }} /></label>;
}

/** Zooms everything inside the widget together; charts redraw at the new size. */
function SizeStepper({ widget, onChange }: { widget: WidgetSpec; onChange: (widget: WidgetSpec) => void }) {
  const scale = widget.contentScale ?? 100;
  const step = (direction: 1 | -1) => {
    const index = WIDGET_CONTENT_SCALES.findIndex(value => value >= scale);
    const next = WIDGET_CONTENT_SCALES[Math.min(WIDGET_CONTENT_SCALES.length - 1, Math.max(0, (index < 0 ? WIDGET_CONTENT_SCALES.length - 1 : index) + direction))];
    if (next !== scale) onChange({ ...widget, contentScale: next });
  };
  return <div className="setting-row"><SettingLabel label="Zoom" hint="Text, icons and charts inside this widget" />
    <div className="size-stepper" role="group" aria-label="Widget size">
      <button type="button" className="icon-button small" aria-label="Smaller" disabled={scale <= WIDGET_CONTENT_SCALES[0]} onClick={() => step(-1)}><Icon name="minus" /></button>
      {scale === 100 ? <span className="size-stepper-value" aria-label={`Size ${scale}%`}>{scale}%</span>
        : <button type="button" className="size-stepper-value" aria-label={`Size ${scale}%, reset to 100%`} data-tooltip="Reset to 100%" onClick={() => onChange({ ...widget, contentScale: 100 })}>{scale}%</button>}
      <button type="button" className="icon-button small" aria-label="Larger" disabled={scale >= WIDGET_CONTENT_SCALES.at(-1)!} onClick={() => step(1)}><Icon name="plus" /></button>
    </div></div>;
}
