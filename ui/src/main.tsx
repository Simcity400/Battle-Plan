import './shell/commitCount';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource-variable/inter';
import '@fontsource-variable/roboto';
import '@fontsource-variable/roboto-condensed';
import '@fontsource-variable/source-serif-4';
import '@fontsource-variable/cascadia-code';
import './styles.css';
import App from './shell/App';
import ExportPresentationApp from './export/ExportPresentationApp';
import Tooltip from './components/Tooltip';
import TooltipLayer from './charts/TooltipLayer';
import { applySavedAppearance } from './shell/appearance/useAppearance';
import { desktopAvailable } from './shell/api';
import { shareRecordingPreferences } from './shell/recordingPreferences';
import { startPerf } from './shell/perf';
import { registerGameWindow } from './dashboard/gameWindows';
import EquipmentDesignerWindow from './dashboard/EquipmentDesignerWindow';
import IndustrialOrganisationWindow from './dashboard/IndustrialOrganisationWindow';
import IntelligenceAgencyWindow from './dashboard/IntelligenceAgencyWindow';

const isExportPresentation = new URLSearchParams(window.location.search).get('export') === '1';
if (!isExportPresentation) {
  registerGameWindow('equipmentDesigner', EquipmentDesignerWindow);
  registerGameWindow('industrialOrganisation', IndustrialOrganisationWindow);
  registerGameWindow('intelligenceAgency', IntelligenceAgencyWindow);
}
applySavedAppearance();
if (!isExportPresentation) {
  void startPerf().catch(error => console.error('Measurement unavailable:', error));
  if (desktopAvailable) void shareRecordingPreferences().catch(error => console.error('Campaign flags stay on this device:', error));
}
const root = document.getElementById('root');
if (!root) throw new Error('The desktop application root is missing.');
createRoot(root).render(
  isExportPresentation
    ? <ExportPresentationApp />
    : <StrictMode><App /><Tooltip /><TooltipLayer /></StrictMode>,
);
