import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '../styles.css';
import Tooltip from '../components/Tooltip';
import { applySavedAppearance } from '../shell/appearance/useAppearance';
import RecorderPanel from './RecorderPanel';

const fonts = applySavedAppearance();
const root = document.getElementById('root');
if (!root) throw new Error('The recorder panel root is missing.');
const app = createRoot(root);
const render = () => app.render(<StrictMode><RecorderPanel /><Tooltip /></StrictMode>);
if (fonts) void fonts.then(render);
else render();
