import { useEffect, useId } from 'react';
import type { ReactNode } from 'react';
import { Icon } from '../components/Icons';
import { Popover, usePopover } from '../components/Popover';
import { resolveWidgetCampaign } from './campaigns';
import { widgetDefinition } from './definitions';
import type { WidgetRuntime } from './definitions';
import type { CountrySelection, DashboardTemplate, WidgetSpec } from './model';
import { CountriesSection } from './settings/CountriesSection';

export default function WidgetCountryControl({ widget, template, runtime, onApply, compact = false, onRegisterOpen, children }: {
  widget: WidgetSpec; template: DashboardTemplate; runtime: WidgetRuntime; onApply: (widget: WidgetSpec, view?: boolean) => void;
  compact?: boolean; onRegisterOpen?: (open: (() => void) | undefined) => void; children: ReactNode;
}) {
  const id = useId();
  const menu = usePopover();
  const definition = widgetDefinition(widget.kind);
  const limits = definition?.countries ?? { max: 1 };
  const SubjectMenu = definition?.subjectMenu;
  const reference = runtime.tracks.find(track => track.id === resolveWidgetCampaign(widget, template, runtime.defaultRoleId).trackIds[0]);

  useEffect(() => {
    if (!onRegisterOpen) return;
    if (!reference) { onRegisterOpen(undefined); return; }
    onRegisterOpen(() => { menu.popup.current?.showPopover(); });
    return () => onRegisterOpen(undefined);
  }, [onRegisterOpen, reference, menu.popup]);

  if (!reference) return children;
  const save = (countries: CountrySelection) => {
    onApply({ ...widget, countries }, true);
    if (limits.max <= 1) menu.close();
  };
  return <>
    <button ref={menu.trigger} type="button" className={`dash-widget-country-button${compact ? ' is-compact' : ''}`} popoverTarget={id}
      aria-haspopup="dialog" aria-expanded={menu.open} aria-label={`${widget.title}: choose countries`}>
      {children}<Icon name="chevron-down" className="dash-widget-country-chevron" />
    </button>
    <Popover menu={menu} id={id} className="widget-countries-menu" aria-label={`${widget.title} countries`}>
      {menu.open && <CountriesSection inline widget={widget} reference={reference} runtime={runtime} limits={limits} onChange={save} />}
      {menu.open && SubjectMenu && <SubjectMenu widget={widget} runtime={runtime} onApply={next => { onApply(next, true); menu.close(); }} />}
    </Popover>
  </>;
}
