import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';

mount(App, { target: document.getElementById('app')! });

// A file dropped outside a drop target must not make the window open (and leave) the page.
for (const t of ['dragover', 'drop'] as const) window.addEventListener(t, (e) => { if (e.dataTransfer?.types.includes('Files')) e.preventDefault(); });
