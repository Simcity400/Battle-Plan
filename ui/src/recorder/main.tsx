import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource-variable/inter';
import '@fontsource-variable/roboto';
import '@fontsource-variable/roboto-condensed';
import '@fontsource-variable/source-serif-4';
import '@fontsource-variable/cascadia-code';
import '../styles.css';
import Tooltip from '../components/Tooltip';
import { applySavedAppearance } from '../shell/appearance/useAppearance';
import RecorderPanel from './RecorderPanel';

applySavedAppearance();
const root = document.getElementById('root');
if (!root) throw new Error('The recorder panel root is missing.');
createRoot(root).render(<StrictMode><RecorderPanel /><Tooltip /></StrictMode>);
