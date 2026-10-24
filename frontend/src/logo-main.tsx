/** 独立台标库页入口（`/logo`）。 */

import { createRoot } from 'react-dom/client';

import { LogosPage } from './logo';
import './style.css';

createRoot(document.getElementById('root')!).render(
  <div className="page">
    <LogosPage />
  </div>,
);
