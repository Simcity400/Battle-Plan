import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import App from './App';
import Tooltip from './Tooltip';
import './styles.css';

const root = document.getElementById('root');
if (!root) throw new Error('The desktop application root is missing.');
createRoot(root).render(<StrictMode><App /><Tooltip /></StrictMode>);
