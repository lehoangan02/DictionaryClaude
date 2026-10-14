import { call } from './api';

export type Tab = 'search' | 'history' | 'quiz' | 'notes' | 'claude';

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
  // The settings side panel is open (it opens from the gear in the top bar, not from a tab).
  settings: false,
  // The left drawer (menu) is open.
  menu: false,
  // Bumped when the dictionary order changes so tabs redraw.
  orderVersion: 0,
});

/** Words visited, like a browser's history; `i` is the current position (== length when on the home screen). */
export const nav = $state({ stack: [] as string[], i: 0 });

/** Words the dictionaries do not have: they never reach History, so they must not stay in the Back trail either. */
const missing = new Set<string>();
export function markMissing(word: string, isMissing: boolean) {
  if (isMissing) missing.add(word); else missing.delete(word);
}

export function open(word: string) {
  app.tab = 'search';
  if (nav.stack[nav.i] === word) { app.word = word; return; }
  // Opening something new drops whatever was "forward" of here, and a word that turned out not to exist.
  const upto = nav.stack.slice(0, nav.i + 1); // through the word on screen (all of it from the home screen)
  nav.stack = [...upto.filter((w) => !missing.has(w) && w !== word), word];
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
export function goBack() { if (canBack()) { nav.i--; replace = true; setTimeout(() => (replace = false), 0); show(); } }
export function goForward() { if (canForward()) { nav.i++; replace = true; setTimeout(() => (replace = false), 0); show(); } }

/** A word picked from the History list: stand on its latest visit in the trail, so Back and Forward are the words around that visit. */
export async function openFromHistory(word: string) {
  await loadTrail(); // the trail as the History list shows it, not as this session's Back/Forward left it
  const j = nav.stack.lastIndexOf(word);
  if (j < 0) { open(word); return; } // older than the trail: treat it as a new lookup
  nav.i = j;
  show();
}

// Back behaves as Android's Navigation component does (bottom navigation with several back stacks):
//  - Home (the Dictionary start screen) is the base of every stack.
//  - Each tab keeps its own stack. Tapping a tab saves the stack you are leaving and restores that tab's own,
//    so tab taps never build loops: Home -> History -> word -> Home leaves just Home.
//  - Opening something from another tab (a word from History) is pushed on that tab's stack, so Back returns to it.
//  - Back pops one place; from a tab's first screen it goes to Home; at Home it leaves the app.
type Place = { tab: Tab; word: string; seg: Tab; close?: () => void };
const BASE: Place = { tab: 'search', word: '', seg: 'search' };
let stack: Place[] = [BASE];
const saved: Partial<Record<Tab, Place[]>> = {};
let replace = false; // the next change steps along the word trail (the arrows): it replaces the place
let armed = false; // the system history holds one spare step, so the Back button reaches us instead of leaving
let leaving = false; // a step is being taken out of the system history; wait for it before touching the history again

const top = () => stack[stack.length - 1];
const same = (p: Place) => p.tab === app.tab && (p.tab !== 'search' || p.word === app.word);

function apply() {
  const t = top();
  app.tab = t.tab;
  if (t.tab === 'search') {
    app.word = t.word;
    const j = t.word ? nav.stack.lastIndexOf(t.word) : -1;
    if (j >= 0) nav.i = j; else if (!t.word) nav.i = nav.stack.length;
  }
}

/** One spare history step exists exactly while there is somewhere to go back to inside the app. */
function arm() {
  if (leaving) return;
  if (stack.length > 1 && !armed) { history.pushState({ lexis: 1 }, ''); armed = true; }
  else if (stack.length <= 1 && armed) { armed = false; leaving = true; history.back(); }
}

export function goTab(t: Tab) {
  const here = top().seg;
  if (t === here) {
    // Already in this tab's stack, perhaps on a word opened from it: tapping the tab goes back to the tab itself.
    while (stack.length > 1 && top().tab !== t) stack.pop()!.close?.();
    apply(); arm();
    return;
  }
  saved[here] = stack.slice(1);
  const own = (saved[t] ?? []);
  saved[t] = undefined;
  // Only the places that belong to the tab itself come back (not a word opened from it that lives in Dictionary).
  while (own.length && own[own.length - 1].tab !== t) own.pop();
  stack = t === 'search' ? [BASE, ...own] : [BASE, ...(own.length ? own : [{ tab: t, word: '', seg: t }])];
  apply(); arm();
}
export function goHome() { stack = [BASE]; for (const k of Object.keys(saved)) saved[k as Tab] = undefined; apply(); arm(); }

// A screen opened on top of a tab (a past session, a photographed note, the camera): Back closes it first.
export function openDetail(close: () => void) {
  stack.push({ ...top(), close });
  arm();
}
export function openSettings() {
  if (app.settings) return;
  app.settings = true;
  openDetail(() => (app.settings = false));
}
export function closeSettings() { closeDetail(); }
export function openMenu() {
  if (app.menu || app.settings) return;
  app.menu = true;
  openDetail(() => (app.menu = false));
}
export function closeMenu() { closeDetail(); }
/** The drawer's Settings button: the drawer gives way to Settings in the same place of the Back trail. */
export function menuToSettings() {
  if (!app.menu) return openSettings();
  app.menu = false; app.settings = true;
  stack[stack.length - 1].close = () => (app.settings = false);
}

/** Closes the open detail the way Back does. */
export function closeDetail() { if (top().close) back(); }

/** Back: one place, as Android does. Returns false when there is nowhere left (the app should leave). */
function back(): boolean {
  if (stack.length <= 1) return false;
  const gone = stack.pop()!;
  if (stack.length === 1 && saved.search?.length) { stack = [BASE, ...saved.search]; saved.search = undefined; }
  gone.close?.();
  apply(); arm();
  return true;
}

/** Records the current tab and word as a place; called whenever either changes by other means than goTab/Back. */
export function trackLocation() {
  const [step] = [replace];
  replace = false;
  if (same(top())) return;
  const here: Place = { tab: app.tab, word: app.tab === 'search' ? app.word : '', seg: top().seg };
  if (step && !top().close && top() !== BASE) stack[stack.length - 1] = here;
  else stack.push(here);
  arm();
}

export function listenForBack() {
  window.addEventListener('popstate', () => {
    if (leaving) { leaving = false; arm(); return; } // our own step out finished; the stack may have changed meanwhile
    // The spare step was used up by this Back press.
    armed = false;
    // A quiz in progress asks to pause instead of letting Back leave it.
    if (app.focus) { arm(); window.dispatchEvent(new Event('lexis:back')); return; }
    if (!back()) return;
  });
  arm();
}

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

/** Re-reads the trail (lookups made on other devices arrive through sync) without moving the current position. */
async function refreshTrail() {
  const word = app.tab === 'search' ? app.word : '';
  await loadTrail();
  if (!word) return;
  const j = nav.stack.lastIndexOf(word);
  if (j >= 0) nav.i = j;
}

export function bump() {
  app.version++;
  refreshTrail();
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
  if (tab === 'settings' as string) openSettings();
  else if (tab) app.tab = tab;
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

// Barron's pronunciation download. It lives here, not in the Settings screen, so it keeps going when you leave
// Settings, and a lost connection makes it wait and try again instead of stopping.
export const audioDl = $state({ running: false, done: 0, total: 0, note: '' });
let stopAudio = false;
const nap = (ms: number) => new Promise((r) => setTimeout(r, ms));
export function stopAudioDownload() { stopAudio = true; }
export async function startAudioDownload() {
  if (audioDl.running) return;
  audioDl.running = true; audioDl.note = ''; stopAudio = false;
  try { localStorage.setItem('audio.want', '1'); } catch { /* private window */ }
  let waitS = 10;
  try {
    while (!stopAudio) {
      let r;
      try { r = await call('audio_prefetch', { limit: 12 }); }
      catch (e) { audioDl.note = `${(e as Error).message}`; break; }
      audioDl.done = r.done; audioDl.total = r.total;
      if (r.done >= r.total) { audioDl.note = 'All recordings are on this device.'; break; }
      if (r.rate_limited) { audioDl.note = 'Waiting politely for the server…'; await nap(((r.retry_after ?? 5) + 2) * 1000); continue; }
      if (r.failed && !r.fetched) {
        // No connection (or a hiccup): wait, then try again, a little longer each time.
        audioDl.note = `No connection, trying again in ${waitS} s. ${r.done} of ${r.total} are saved.`;
        for (let i = 0; i < waitS && !stopAudio; i++) await nap(1000);
        waitS = Math.min(waitS * 2, 120);
        continue;
      }
      waitS = 10; audioDl.note = '';
    }
  } finally {
    if (stopAudio || audioDl.note.startsWith('All')) { try { localStorage.removeItem('audio.want'); } catch { /* private window */ } }
    audioDl.running = false; if (stopAudio && !audioDl.note.startsWith('All')) audioDl.note = 'Stopped. It resumes where it left off.'; }
}

// Downloads the person asked for but that were cut off (leaving the app, losing the connection): start them again
// when the app comes back, at launch, and every so often while it is open. They continue from where they stopped.
let resuming = false;
export async function resumeDownloads() {
  try { if (localStorage.getItem('audio.want') && !audioDl.running) startAudioDownload(); } catch { /* private window */ }
  if (resuming) return;
  resuming = true;
  try {
    const [list, prog] = await Promise.all([call('packs_list'), call('pack_progress')]);
    if (!list.paired) return;
    for (const p of list.packs) {
      if (p.want && !(p.id in prog)) call('pack_install', { id: p.id }).then(() => call('sources').then((s) => { app.sources = s; bump(); })).catch(() => {});
    }
  } catch { /* offline or busy: next time */ } finally { resuming = false; }
}
export function keepDownloadsGoing() {
  document.addEventListener('visibilitychange', () => { if (!document.hidden) resumeDownloads(); });
  window.addEventListener('online', () => resumeDownloads());
  setInterval(resumeDownloads, 20000);
  resumeDownloads();
}
