import './shell/commitCount';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import './styles.css';
import App from './shell/App';
import { loadable } from './components/loadable';
import Tooltip from './components/Tooltip';
import TooltipLayer from './charts/TooltipLayer';
import { applySavedAppearance } from './shell/appearance/useAppearance';
import { desktopAvailable } from './shell/api';
import { shareRecordingPreferences } from './shell/recordingPreferences';
import { startPerf } from './shell/perf';
import { registerGameWindow } from './dashboard/gameWindows';

const isExportPresentation = new URLSearchParams(window.location.search).get('export') === '1';
const ExportPresentationApp = loadable(() => import('./export/ExportPresentationApp').then(module => module.default));
if (!isExportPresentation) {
  registerGameWindow('equipmentDesigner', loadable(() => import('./dashboard/EquipmentDesignerWindow').then(module => module.default)));
  registerGameWindow('industrialOrganisation', loadable(() => import('./dashboard/IndustrialOrganisationWindow').then(module => module.default)));
  registerGameWindow('intelligenceAgency', loadable(() => import('./dashboard/IntelligenceAgencyWindow').then(module => module.default)));
}
const fonts = applySavedAppearance();
if (!isExportPresentation) {
  void startPerf().catch(error => console.error('Measurement unavailable:', error));
  if (desktopAvailable) void shareRecordingPreferences().catch(error => console.error('Campaign flags stay on this device:', error));
}
const root = document.getElementById('root');
if (!root) throw new Error('The desktop application root is missing.');
const app = createRoot(root);
const render = () => app.render(
  isExportPresentation
    ? <ExportPresentationApp />
    : <StrictMode><App /><Tooltip /><TooltipLayer /></StrictMode>,
);
if (fonts) void fonts.then(render);
else render();
