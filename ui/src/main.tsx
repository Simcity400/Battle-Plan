import './shell/commitCount';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource-variable/inter';
import '@fontsource-variable/roboto';
import '@fontsource-variable/roboto-condensed';
import '@fontsource-variable/source-serif-4';
import '@fontsource-variable/cascadia-code';
// The design system loads first so component stylesheets can refine it.
import './styles.css';
import App from './shell/App';
import Tooltip from './components/Tooltip';
import TooltipLayer from './charts/TooltipLayer';
import { applySavedAppearance } from './shell/appearance/useAppearance';
import { desktopAvailable } from './shell/api';
import { shareRecordingPreferences } from './shell/recordingPreferences';
import { startPerf } from './shell/perf';
import { FrameCommits } from './dashboard/frameUpdates';

applySavedAppearance();
void startPerf().catch(error => console.error('Measurement unavailable:', error));
if (desktopAvailable) void shareRecordingPreferences().catch(error => console.error('Campaign flags stay on this device:', error));
const root = document.getElementById('root');
if (!root) throw new Error('The desktop application root is missing.');
createRoot(root).render(<StrictMode><FrameCommits /><App /><Tooltip /><TooltipLayer /></StrictMode>);
