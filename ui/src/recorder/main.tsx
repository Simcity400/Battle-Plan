import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource-variable/inter';
import '@fontsource-variable/roboto';
import '@fontsource-variable/roboto-condensed';
import '@fontsource-variable/source-serif-4';
import '@fontsource-variable/cascadia-code';
// The design system loads first so the panel's stylesheet can refine it.
import '../styles.css';
import Tooltip from '../components/Tooltip';
import { applySavedAppearance } from '../shell/appearance/useAppearance';
import RecorderPanel from './RecorderPanel';

// The panel shares the replay app's WebView storage, so this is the user's appearance.
applySavedAppearance();
const root = document.getElementById('root');
if (!root) throw new Error('The recorder panel root is missing.');
createRoot(root).render(<StrictMode><RecorderPanel /><Tooltip /></StrictMode>);
