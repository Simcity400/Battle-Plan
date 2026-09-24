import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import '@fontsource-variable/inter';
// The design system loads first so component stylesheets can refine it.
import './styles.css';
import App from './shell/App';
import Tooltip from './components/Tooltip';
import { applySavedAppearance } from './shell/appearance/useAppearance';

applySavedAppearance();
const root = document.getElementById('root');
if (!root) throw new Error('The desktop application root is missing.');
createRoot(root).render(<StrictMode><App /><Tooltip /></StrictMode>);
