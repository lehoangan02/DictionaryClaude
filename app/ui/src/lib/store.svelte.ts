import { call } from './api';

export type Tab = 'search' | 'history' | 'quiz' | 'notes' | 'settings';

export const app = $state({
  tab: 'search' as Tab,
  word: '',
  source: 'wiktionary',
  sources: [] as { id: string; name: string; entries: number | null }[],
  theme: (localStorage.getItem('theme') ?? 'system') as 'system' | 'light' | 'dark',
  // Bumped when history/notes change so dependent views refresh.
  version: 0,
  // Unit to pre-select when the quiz tab opens (from a word's Barron's card).
  quizUnit: null as number | null,
  // A quiz is running: hide the navigation so nothing covers the answer buttons.
  focus: false,
  // Bumped when the dictionary order changes so tabs redraw.
  orderVersion: 0,
});

/** Words visited, like a browser's history; `i` is the current position (== length when on the home screen). */
export const nav = $state({ stack: [] as string[], i: 0 });

export function open(word: string) {
  app.tab = 'search';
  if (nav.stack[nav.i] === word) { app.word = word; return; }
  // Opening something new drops whatever was "forward" of here.
  nav.stack = [...nav.stack.slice(0, nav.i), word];
  nav.i = nav.stack.length - 1;
  app.word = word;
}

export const canBack = () => nav.i > 0;
export const canForward = () => nav.i < nav.stack.length - 1;
export const backWord = () => (nav.i > 0 ? nav.stack[nav.i - 1] : null);
export const forwardWord = () => (nav.i < nav.stack.length - 1 ? nav.stack[nav.i + 1] : null);

function show() {
  app.tab = 'search';
  app.word = nav.stack[nav.i] ?? '';
}
export function goBack() { if (canBack()) { nav.i--; show(); } }
export function goForward() { if (canForward()) { nav.i++; show(); } }

/** Recent lookups (oldest first) so Back works across restarts. */
export async function loadTrail() {
  try {
    const rows: { word: string }[] = await call('history_list', { limit: 40 });
    const words: string[] = [];
    for (const r of rows.reverse()) if (words.at(-1) !== r.word) words.push(r.word);
    nav.stack = words;
    nav.i = words.length; // on home: Back goes to the most recent word
  } catch { /* history is optional */ }
}

export function bump() {
  app.version++;
}

export function setTheme(t: 'system' | 'light' | 'dark') {
  app.theme = t;
  localStorage.setItem('theme', t);
  applyTheme();
}

export function applyTheme() {
  const root = document.documentElement;
  if (app.theme === 'system') root.removeAttribute('data-theme');
  else root.setAttribute('data-theme', app.theme);
}

export async function boot() {
  applyTheme();
  app.sources = await call('sources');
  const saved = localStorage.getItem('source');
  const vis = orderedSources().visible;
  if (saved && vis.some((s) => s.id === saved)) app.source = saved;
  else app.source = vis[0]?.id ?? app.sources[0]?.id ?? 'wiktionary';
  await loadTrail();
  // Deep links: ?w=abate&tab=quiz&source=wordnet
  const q = new URLSearchParams(location.search);
  const src = q.get('source');
  if (src && app.sources.some((s) => s.id === src)) app.source = src;
  const tab = q.get('tab') as Tab | null;
  if (tab) app.tab = tab;
  const w = q.get('w');
  if (w) open(w);
}

const ORDER_KEY = 'source-order';
/** The dictionaries in the person's chosen order, hidden ones left out. */
export function orderedSources() {
  let saved: { order: string[]; hidden: string[] } = { order: [], hidden: [] };
  try { saved = { order: [], hidden: [], ...JSON.parse(localStorage.getItem(ORDER_KEY) ?? '{}') }; } catch { /* fresh */ }
  const rank = (id: string) => { const i = saved.order.indexOf(id); return i < 0 ? 999 : i; };
  const all = [...app.sources].sort((a, b) => rank(a.id) - rank(b.id));
  return { all, hidden: saved.hidden, visible: all.filter((s) => !saved.hidden.includes(s.id)) };
}
export function saveOrder(order: string[], hidden: string[]) {
  localStorage.setItem(ORDER_KEY, JSON.stringify({ order, hidden }));
  app.orderVersion++;
  // The first visible dictionary is the default.
  const first = order.find((id) => !hidden.includes(id));
  if (first) setSource(first);
}

export function setSource(id: string) {
  app.source = id;
  localStorage.setItem('source', id);
}
