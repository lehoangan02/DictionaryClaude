import { call } from './api';

export type Tab = 'search' | 'history' | 'quiz' | 'notes' | 'settings';

export const app = $state({
  tab: 'search' as Tab,
  word: '',
  source: 'wiktionary',
  sources: [] as { id: string; name: string; entries: number }[],
  theme: (localStorage.getItem('theme') ?? 'system') as 'system' | 'light' | 'dark',
  // Bumped when history/notes change so dependent views refresh.
  version: 0,
  // Unit to pre-select when the quiz tab opens (from a word's Barron's card).
  quizUnit: null as number | null,
  // A quiz is running: hide the navigation so nothing covers the answer buttons.
  focus: false,
});

export function open(word: string) {
  app.word = word;
  app.tab = 'search';
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
  if (saved && app.sources.some((s) => s.id === saved)) app.source = saved;
  else app.source = app.sources[0]?.id ?? 'wiktionary';
  // Deep links: ?w=abate&tab=quiz&source=wordnet
  const q = new URLSearchParams(location.search);
  const src = q.get('source');
  if (src && app.sources.some((s) => s.id === src)) app.source = src;
  const tab = q.get('tab') as Tab | null;
  if (tab) app.tab = tab;
  const w = q.get('w');
  if (w) open(w);
}

export function setSource(id: string) {
  app.source = id;
  localStorage.setItem('source', id);
}
