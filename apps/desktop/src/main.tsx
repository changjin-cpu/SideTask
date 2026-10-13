import React, { type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { isDesktop } from './lib/native';
import './styles/app.css';
import './styles/variants.css';

const surface = new URLSearchParams(location.search).get('surface') || 'console';
document.documentElement.dataset.surface = surface;
document.documentElement.dataset.runtime = isDesktop ? 'desktop' : 'browser';

async function loadApplication(): Promise<ReactNode> {
  if (surface === 'mobile') {
    const { default: MobileApplication } = await import('./surfaces/mobile/Mobile');
    return <MobileApplication />;
  }
  if (surface === 'edge-handle') {
    const { default: HandleApplication } = await import('./startup/HandleApplication');
    return <HandleApplication />;
  }
  if (surface === 'edge-panel') {
    const { default: PanelApplication } = await import('./startup/PanelApplication');
    return <PanelApplication />;
  }
  const [{ default: ConsoleApplication }, preview] = await Promise.all([
    import('./startup/ConsoleApplication'),
    isDesktop ? undefined : import('./startup/BrowserEdgePreview'),
  ]);
  const Preview = preview?.default;
  return <ConsoleApplication preview={Preview ? <Preview /> : undefined} />;
}

const root = createRoot(document.getElementById('root')!);
// Load the chosen surface completely before mounting. Suspending Console under
// an already mounted bridge could discard navigation before its handlers exist.
void loadApplication().then(application => {
  root.render(<React.StrictMode>{application}</React.StrictMode>);
}).catch(reason => {
  console.error('SideTask could not load its interface', reason);
  // No store or draft has mounted yet, so reloading is safe after an import fails.
  root.render(surface === 'edge-handle'
    ? <button className="edge-handle" aria-label="侧笺未能加载，点击重试" title="侧笺未能加载，点击重试" onClick={() => location.reload()}>↻</button>
    : <main className="app-loading"><p role="alert">界面未能加载，请重试。</p><button className="secondary-button" onClick={() => location.reload()}>重新加载</button></main>);
});
