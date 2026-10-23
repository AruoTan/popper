import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { CaptureApp } from './CaptureApp';
import { PinApp } from './PinApp';
import './capture.css';
const root = document.getElementById('root');
if (!root) throw new Error('Missing screen capture root');
const query = new URLSearchParams(window.location.search), mode = query.get('mode');
createRoot(root).render(<StrictMode>{mode === 'pin' ? <PinApp /> : mode === 'notice' ? <div className="capture-notice" role="status">{query.get('message')}</div> : <CaptureApp />}</StrictMode>);
