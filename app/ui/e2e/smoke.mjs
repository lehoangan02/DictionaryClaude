// Drives the real UI in the installed Chrome against a running gredict-server.
//   cargo run -p gredict-server &   (with packs built)   then   npm run e2e
import puppeteer from 'puppeteer-core';
import { mkdirSync } from 'node:fs';
import { execFileSync } from 'node:child_process';

const BASE = process.env.BASE ?? 'http://127.0.0.1:8787/';
const OUT = process.env.SHOTS ?? '/tmp/shots';
mkdirSync(OUT, { recursive: true });

const browser = await puppeteer.launch({
  executablePath: process.env.CHROME ?? '/usr/bin/google-chrome',
  headless: 'new',
  args: ['--no-sandbox', '--disable-gpu'],
});

let failed = 0;
const ok = (cond, msg) => { console.log(`${cond ? 'PASS' : 'FAIL'}  ${msg}`); if (!cond) failed++; };
const text = (page, sel) => page.$eval(sel, (e) => e.textContent.trim()).catch(() => null);
// Headwords show syllables (a·bate); compare the plain word.
const plain = async (page, sel) => ((await text(page, sel)) ?? '').replace(/·/g, '');

async function newPage(w, h) {
  const page = await browser.newPage();
  await page.setViewport({ width: w, height: h, deviceScaleFactor: 1 });
  page.on('pageerror', (e) => { console.log('PAGEERROR', e.message); failed++; });
  page.on('console', (m) => { if (m.type() === 'error') console.log('CONSOLE', m.text()); });
  return page;
}
const clickText = async (page, sel, label) => {
  const els = await page.$$(sel);
  for (const el of els) {
    if ((await el.evaluate((e) => e.textContent.trim())).includes(label)) { await el.click(); return true; }
  }
  return false;
};

// ---- desktop: search, entry, notes -> thesaurus ----
const d = await newPage(1100, 1000);
await d.goto(BASE, { waitUntil: 'networkidle0' });
await d.type('input[type=search]', 'abat');
await d.waitForSelector('.dd li button');
ok((await text(d, '.dd li button span')) === 'abate', 'autosuggest puts the GRE word "abate" first');
await d.keyboard.press('Enter');
await d.waitForSelector('.hw');
ok(['abate', 'abat'].includes(await plain(d, '.hw')), 'Enter opens an entry');
await d.goto(BASE + '?w=abate', { waitUntil: 'networkidle0' });
await d.waitForSelector('.hw');
ok((await plain(d, '.hw')) === 'abate', 'deep link opens the entry');
ok((await text(d, '.hw')) === 'a·bate', 'the headword shows its syllables');
ok(!!(await d.$('.gre')), "Barron's card is shown for a GRE word");
ok((await d.$$('.senses > li')).length >= 2, 'several senses listed');
await clickText(d, '.srcs button', 'WordNet');
await d.waitForFunction(() => document.querySelector('.srcs button.on')?.textContent.includes('WordNet'));
// While the WordNet entry loads the body shows a spinner (not a dimmed article), so wait for both to go.
await d.waitForFunction(() => !document.querySelector('article.dim') && !document.querySelector('.wait'));
ok((await d.$$('.senses > li')).length >= 1, 'WordNet source shows senses');

// A dictionary without the word is greyed out and cannot be opened; the word still shows from one that has it.
await d.goto(BASE + '?w=selfie&source=wordnet', { waitUntil: 'networkidle0' });
await d.waitForSelector('.hw');
const tabState = await d.$$eval('.srcs button', (b) => b.map((x) => ({ t: x.textContent.trim(), off: x.disabled, on: x.classList.contains('on') })));
ok(tabState.find((t) => t.t === 'WordNet')?.off && tabState.find((t) => t.t === 'Webster 1913')?.off, 'dictionaries without the word are disabled');
ok(tabState.find((t) => t.t === 'Wiktionary')?.on && !tabState.find((t) => t.t === 'Wiktionary')?.off, 'the result shows from the dictionary that has it');
ok(!(await d.$('.note')), 'no "not in this dictionary" panel');
await d.goto(BASE, { waitUntil: 'networkidle0' });
// The server popover closes when you press anywhere else, and keeps clear of the search box.
await d.click('.pill'); await d.waitForSelector('.pop');
const gap = await d.evaluate(() => document.querySelector('.pop').getBoundingClientRect().top - document.querySelector('.sb input').getBoundingClientRect().bottom);
ok(gap >= 10, `the popover keeps clear of the search box (${Math.round(gap)} px gap)`);
await d.mouse.click(300, 600);
await new Promise((r) => setTimeout(r, 250));
ok(!(await d.$('.pop')), 'the popover closes when you click elsewhere');
await d.click('.pill'); await d.waitForSelector('.pop'); await d.keyboard.press('Escape');
await new Promise((r) => setTimeout(r, 250));
ok(!(await d.$('.pop')), 'the popover closes on Escape');

// Back and forward go through the words you looked up, not the alphabet.
await d.type('input[type=search]', 'x');
await d.$eval('input[type=search]', (e) => { e.value = ''; });
await d.evaluate(() => (document.getElementById('lexis-q').value = ''));
await d.goto(BASE + '?w=lucid', { waitUntil: 'networkidle0' });
await d.waitForSelector('.hw');
const nav1 = await d.$$eval('.hn button', (b) => b.map((x) => ({ t: x.textContent.trim(), off: x.disabled })));
ok(nav1.length === 2, 'a back/forward strip is shown over the entry');
await d.evaluate(() => document.querySelectorAll('.hn button')[0]?.click());
await d.waitForSelector('.hw');

// Webster 1913 has no IPA of its own: the CMU pronunciation fills in.
await d.goto(BASE + '?w=obdurate&source=gcide', { waitUntil: 'networkidle0' });
await d.waitForSelector('.hw');
ok((await d.$$eval('.srcs button', (b) => b.map((x) => x.textContent.trim()))).includes('Webster 1913'), 'Webster 1913 is a selectable source');
ok(/ˈɑbdɚət/.test(await text(d, '.prons')), 'IPA fallback is shown for a Webster entry');
await d.goto(BASE + '?w=abate', { waitUntil: 'networkidle0' });
await d.waitForSelector('.hw');

// Notes: write relations, see them parsed, save.
await clickText(d, '.desk button', 'Notes');
await d.waitForSelector('textarea');
await d.type('textarea', 'abate = lessen = subside\nabate >< intensify\nimbibition ~= osmosis');
await d.waitForFunction(() => document.querySelectorAll('.pv .pill').length >= 5);
const ta = await d.$eval('textarea', (e) => e.value);
ok(ta.includes('≈'), '"~=" auto-converts to ≈');
ok((await d.$$('.pv .pill')).length === 5, 'five relations are parsed live (3 synonym pairs, 1 opposite, 1 similar)');
await d.screenshot({ path: `${OUT}/notes-desktop.png` });
await clickText(d, 'button', 'Save note');
await d.waitForFunction(() => document.querySelectorAll('.nt').length >= 1);
ok(true, 'note saved');

// The thesaurus panel now shows the user's notes.
await d.goto(BASE + '?w=subside', { waitUntil: 'networkidle0' });
await d.waitForSelector('.box.mine');
const mine = await d.$eval('.box.mine', (e) => e.textContent);
ok(mine.includes('abate') && mine.includes('lessen') && mine.includes('intensify'), 'thesaurus shows synonyms and opposite from my notes');
await d.screenshot({ path: `${OUT}/thesaurus-desktop.png`, fullPage: true });

// History recorded the lookups.
await clickText(d, '.desk button', 'History');
await d.waitForSelector('.list li');
ok((await d.$$('.list li')).length >= 2, 'history lists lookups');

// ---- mobile: quiz flow ----
const m = await newPage(390, 844);
await m.goto(BASE + '?tab=quiz', { waitUntil: 'networkidle0' });
await m.waitForSelector('.units');
ok((await m.$$('[aria-label="Units"] button')).length === 80, 'all 80 Barron units are listed');
await m.screenshot({ path: `${OUT}/quiz-hub-mobile.png`, fullPage: true });
await clickText(m, '.card button', 'Start');
await m.waitForSelector('.ch');
await m.screenshot({ path: `${OUT}/quiz-play-mobile.png` });
for (let n = 0; n < 15; n++) {
  const btns = await m.$$('.ch');
  if (!btns.length) break;
  await btns[0].click(); // first option: some right, some wrong
  await clickText(m, '.go', 'Check answer'); // answers are selected first, then checked
  try { await m.waitForSelector('.fb', { timeout: 5000 }); }
  catch { console.log('NO FEEDBACK. page shows:\n' + (await m.$eval('body', (e) => e.innerText.slice(0, 500)))); throw new Error('no feedback'); }
  await clickText(m, '.go', n < 14 ? 'Next' : 'See results');
  // Wait for the next question (feedback gone) or the result screen.
  await m.waitForFunction(() => !document.querySelector('.fb') || document.querySelector('.ring'));
  if (await m.$('.ring')) break;
}
await m.waitForSelector('.ring');
ok(!!(await m.$('.ring')), 'quiz finishes with a score ring');
await m.screenshot({ path: `${OUT}/quiz-result-mobile.png`, fullPage: true });
await clickText(m, 'button', 'Done');
await m.waitForSelector('.srow');
ok((await m.$$('.srow')).length >= 1, 'finished session appears in history');
await m.type('input[type=search]', 'wrong');
await new Promise((r) => setTimeout(r, 700));
ok((await m.$$('.srow')).length >= 1, 'retrieval search "wrong" finds the session with mistakes');
await m.screenshot({ path: `${OUT}/quiz-sessions-mobile.png`, fullPage: true });

// ---- motion: screens animate in, and reduced motion switches it off ----
{
  const m1 = await newPage(1000, 800);
  await m1.goto(BASE, { waitUntil: 'networkidle0' });
  await m1.evaluate(() => [...document.querySelectorAll('.desk button')].find((b) => b.textContent.includes('Quizzes')).click());
  // Sample the new screen's opacity right after the switch and again once it has settled.
  const early = await m1.evaluate(() => new Promise((r) => requestAnimationFrame(() => setTimeout(() => r(+getComputedStyle(document.querySelector('.view')).opacity), 40))));
  await new Promise((r) => setTimeout(r, 600));
  const late = await m1.evaluate(() => +getComputedStyle(document.querySelector('.view')).opacity);
  ok(early < 1 && late === 1, `a new screen fades in (opacity ${early.toFixed(2)} -> ${late})`);
  const tr = await m1.evaluate(() => getComputedStyle(document.querySelector('.desk button')).transitionDuration);
  ok(tr !== '0s', 'buttons animate their state changes');
  await m1.close();

  const m2 = await browser.newPage();
  await m2.setViewport({ width: 1000, height: 800 });
  await m2.emulateMediaFeatures([{ name: 'prefers-reduced-motion', value: 'reduce' }]);
  await m2.goto(BASE, { waitUntil: 'networkidle0' });
  await m2.evaluate(() => [...document.querySelectorAll('.desk button')].find((b) => b.textContent.includes('Quizzes')).click());
  const reduced = await m2.evaluate(() => new Promise((r) => requestAnimationFrame(() => setTimeout(() => r(+getComputedStyle(document.querySelector('.view')).opacity), 40))));
  ok(reduced === 1, 'with "reduce motion" nothing fades or slides');
  await m2.close();
}

// ---- dictionary order in Settings ----
const o = await newPage(1000, 900);
await o.goto(BASE, { waitUntil: 'networkidle0' });
await o.click('button[aria-label="Menu"]');
await o.waitForSelector('.drawer .item');
ok((await o.$eval('.drawer', (e) => e.textContent)).includes('Settings'), 'the left drawer has a Settings button');
await o.click('.drawer .item');
await o.waitForSelector('.panel');
await o.waitForSelector('.drawer', { hidden: true, timeout: 3000 }).catch(() => {});
ok(!(await o.$('.drawer')), 'Settings replaces the drawer');
await o.goto(BASE + '?tab=settings', { waitUntil: 'networkidle0' });
await o.waitForSelector('.order li');
const names0 = await o.$$eval('.order .nm b', (b) => b.map((x) => x.textContent));
await o.$$eval('.order li', (li) => [...li].find((x) => x.textContent.includes('Webster'))?.querySelectorAll('.mv')[0].click());
await o.waitForFunction((n) => [...document.querySelectorAll('.order .nm b')].map((x) => x.textContent).join() !== n, {}, names0.join());
const names1 = await o.$$eval('.order .nm b', (b) => b.map((x) => x.textContent));
ok(names1.indexOf('Webster 1913') === names0.indexOf('Webster 1913') - 1, 'a dictionary moves up in the order');
await o.goto(BASE + '?w=obdurate', { waitUntil: 'networkidle0' });
await o.waitForSelector('.srcs button');
const tabsNow = await o.$$eval('.srcs button', (b) => b.map((x) => x.textContent.trim()));
ok(tabsNow.join() === names1.filter((n) => tabsNow.includes(n)).join(), 'the tabs follow the chosen order');
await o.evaluate(() => localStorage.removeItem('source-order'));
await o.close();

// ---- the pretest: pick-two, multi-blank, and reading passages ----
await m.bringToFront();
await m.goto(BASE + '?tab=quiz', { waitUntil: 'networkidle0' });
await m.waitForSelector('.units');
ok(await clickText(m, 'button', 'Pretest'), 'pretest is listed under Reviews');
await m.waitForSelector('.ch');
ok((await text(m, '.hint')) === 'Select 2 answers', 'sentence-equivalence asks for two answers');
let sawPassage = false, sawBlanks = false;
for (let n = 1; n <= 20; n++) {
  if (await m.$('details.passage')) sawPassage = true;
  if ((await m.$$('h3')).length >= 2) sawBlanks = true;
  if (n === 15) await m.screenshot({ path: `${OUT}/quiz-reading-mobile.png` });
  if (n === 1) await m.screenshot({ path: `${OUT}/quiz-pick-two-mobile.png` });
  if (await m.$('.hint')) {
    const c = await m.$$('.ch'); await c[0].click(); await c[1].click();
    await clickText(m, '.go', 'Check answer');
  } else if ((await m.$$('h3')).length >= 2) {
    for (const g of await m.$$('.choices')) await (await g.$('.ch')).click();
    await clickText(m, '.go', 'Check answer');
  } else {
    await (await m.$('.ch')).click();
    await clickText(m, '.go', 'Check answer');
  }
  try { await m.waitForSelector('.fb', { timeout: 4000 }); }
  catch { console.log(`NO FEEDBACK at pretest question ${n}:\n` + (await m.$eval('body', (e) => e.innerText.slice(0, 700)))); throw new Error('stuck'); }
  await clickText(m, '.go', n < 20 ? 'Next' : 'See results');
  await m.waitForFunction(() => !document.querySelector('.fb') || document.querySelector('.ring'));
}
ok(sawBlanks, 'multi-blank questions show one choice list per blank');
ok(sawPassage, 'reading questions show their passage');
await m.waitForSelector('.ring');
ok(/\/ 20/.test(await m.$eval('body', (e) => e.innerText)), 'the pretest finishes with a score out of 20');

// ---- OALD speaks with the Oxford recordings, not the system voice ----
{
  const o = await newPage(1100, 800);
  await o.goto(BASE + '?w=laconic&source=oald', { waitUntil: 'networkidle0' });
  await o.waitForSelector('.hw');
  const buttons = await o.$$('.prons .pron');
  ok(buttons.length === 2, 'an OALD entry shows a UK and a US pronunciation');
  for (const [i, accent] of [[0, 'gb'], [1, 'us']]) {
    const [resp] = await Promise.all([o.waitForResponse((r) => r.url().endsWith('/api/audio_get')), buttons[i].click()]);
    const asked = JSON.parse(resp.request().postData()).url;
    ok(new RegExp(`^oald:laconic#.*_${accent}_\\d+$`).test(asked), `the ${accent.toUpperCase()} button asks for the Oxford recording (${asked})`);
    if (process.env.HAVE_OALD_AUDIO) {
      const { data_url } = await resp.json();
      const secs = await o.evaluate((u) => new Promise((res) => { const a = new Audio(u); a.onloadedmetadata = () => res(a.duration); a.onerror = () => res(-1); }), data_url);
      ok(secs > 0.3 && secs < 3, `that recording plays and lasts like one word (${secs.toFixed(2)} s)`);
    }
  }
}

// ---- browser-style keyboard: Ctrl/Cmd+L, Ctrl/Cmd+F, arrows, History ----
{
  const k = await newPage(1100, 700);
  const chord = async (mod, key) => { await k.keyboard.down(mod); await k.keyboard.press(key); await k.keyboard.up(mod); };
  const hw = () => plain(k, '.hw');
  const hwIs = (w) => k.waitForFunction((x) => document.querySelector('.hw')?.textContent.replace(/·/g, '') === x, {}, w);
  for (const w of ['taciturn', 'loquacious', 'abate']) {
    await k.goto(BASE + '?w=' + w, { waitUntil: 'networkidle0' });
    await k.waitForSelector('.hw');
  }

  // A word opened from History stands on its latest visit: Back and Forward are the words around it.
  await clickText(k, '.desk button', 'History');
  await k.waitForSelector('.list .w');
  await clickText(k, '.list .w', 'loquacious');
  await hwIs('loquacious');
  ok((await text(k, '.hn button:first-child span')) === 'taciturn' && (await text(k, '.hn button.r span')) === 'abate',
    'a word opened from History has the words around its latest visit as Back and Forward');

  // Ctrl+L / Cmd+L: the search box, text selected.
  const selected = () => k.evaluate(() => { const a = document.activeElement; return a?.id === 'lexis-q' && a.value.length > 0 && a.selectionStart === 0 && a.selectionEnd === a.value.length; });
  await chord('Control', 'l');
  ok(await selected(), 'Ctrl+L focuses the search box and selects its text');
  await k.keyboard.press('ArrowLeft');
  ok((await hw()) === 'loquacious', 'Left arrow while typing moves the caret, not the history');
  await k.evaluate(() => document.activeElement.blur());
  await chord('Meta', 'l');
  ok(await selected(), 'Cmd+L does the same');
  await k.evaluate(() => document.activeElement.blur());

  // Left / Right: history, like Back and Forward.
  await k.keyboard.press('ArrowLeft');
  await hwIs('taciturn');
  ok(true, 'Left arrow goes back to the previous word');
  await k.keyboard.press('ArrowRight');
  await hwIs('loquacious');
  ok(true, 'Right arrow goes forward again');
  // Opening from History and stepping with the arrows are visits, not lookups: History keeps its order, so the arrows still match it.
  const order = await k.evaluate(async () => (await (await fetch('/api/history_list', { method: 'POST', body: '{"limit":3}' })).json()).map((r) => r.word).join(' '));
  ok(order === 'abate loquacious taciturn', `History is unchanged by the History list and the arrows (${order})`);
  ok((await text(k, '.hn button:first-child span')) === 'taciturn' && (await text(k, '.hn button.r span')) === 'abate',
    'the arrows are the History neighbours of the word on screen');

  // Up / Down scroll the page.
  await k.keyboard.press('ArrowDown');
  await k.waitForFunction(() => scrollY > 0, { timeout: 4000 }).catch(() => {});
  const y1 = await k.evaluate(() => scrollY);
  ok(y1 > 0, 'Down arrow scrolls the page');
  await k.keyboard.press('ArrowUp');
  await k.waitForFunction((y) => scrollY < y, { timeout: 4000 }, y1).catch(() => {});
  ok((await k.evaluate(() => scrollY)) < y1, 'Up arrow scrolls back up');

  // Ctrl+F / Cmd+F: find on page.
  const state = () => k.evaluate(() => ({
    bar: !!document.querySelector('[role=search] input'),
    focus: document.activeElement?.closest('[role=search]') != null,
    n: document.querySelector('.findbar .n')?.textContent.trim() ?? '',
    painted: CSS.highlights.get('find')?.size ?? 0,
    current: CSS.highlights.get('find-cur')?.size ?? 0,
  }));
  await chord('Control', 'f');
  await k.waitForSelector('[role=search] input');
  ok((await state()).focus, 'Ctrl+F opens the find bar with the cursor in it');
  await k.keyboard.type('the');
  let s = await state();
  const total = Number((s.n.match(/of (\d+)/) ?? [])[1] ?? 0);
  ok(/^1 of \d+$/.test(s.n) && total >= 2, `find counts the matches (${s.n})`);
  ok(s.painted === total && s.current === 1, 'every match is highlighted and the current one stands out');
  await k.keyboard.press('Enter');
  ok((await state()).n === `2 of ${total}`, 'Enter goes to the next match');
  await k.keyboard.down('Shift'); await k.keyboard.press('Enter'); await k.keyboard.up('Shift');
  await k.keyboard.down('Shift'); await k.keyboard.press('Enter'); await k.keyboard.up('Shift');
  ok((await state()).n === `${total} of ${total}`, 'Shift+Enter goes back, wrapping round to the last match');
  await k.keyboard.press('ArrowLeft');
  ok((await hw()) === 'loquacious', 'arrows inside the find box do not walk the history');
  await k.keyboard.down('Control'); await k.keyboard.press('a'); await k.keyboard.up('Control');
  await k.keyboard.type('zzqqxx');
  ok((await state()).n === 'No matches', 'find says when nothing matches');
  await k.keyboard.press('Escape');
  s = await state();
  ok(!s.bar && s.painted === 0, 'Escape closes the find bar and clears the highlights');
  await chord('Meta', 'f');
  await k.waitForSelector('[role=search] input');
  ok((await state()).focus, 'Cmd+F opens it too');
  await k.screenshot({ path: `${OUT}/find.png` });
  await k.keyboard.press('Escape');

  // The phone's Back button: a word opened from History goes back to History, then to where you were.
  const tabNow = () => k.evaluate(() => document.querySelector('.desk button.on')?.textContent.trim());
  const back = async (want) => { await k.evaluate(() => history.back()); await k.waitForFunction((w) => document.querySelector('.desk button.on')?.textContent.trim() === w, { timeout: 4000 }, want).catch(() => {}); };
  ok((await tabNow()) === 'Dictionary', 'on the Dictionary after opening a word from History');
  await back('History');
  ok((await tabNow()) === 'History', 'Back from a word opened in History returns to History, not home');
  await back('Dictionary');
  ok((await tabNow()) === 'Dictionary', 'Back again returns to the Dictionary where you were');
  // Tabs keep their own stacks, so tapping them never builds a loop: one Back from History returns to the Dictionary.
  await clickText(k, '.desk button', 'History');
  await clickText(k, '.desk button', 'Dictionary');
  await clickText(k, '.desk button', 'History');
  await back('Dictionary');
  ok((await tabNow()) === 'Dictionary', 'after tapping tabs back and forth, one Back from History is the Dictionary');
  // And a word opened from History: Back is History, Back again is where you were.
  await clickText(k, '.desk button', 'History');
  await k.waitForSelector('.list .w');
  await clickText(k, '.list .w', 'loquacious');
  await k.waitForFunction(() => document.querySelector('.desk button.on')?.textContent.trim() === 'Dictionary', { timeout: 4000 });
  await back('History');
  ok((await tabNow()) === 'History', 'Back from a word opened in History returns to History again');
}

// ---- two devices through a real sync server ----
const BASE_B = process.env.BASE_B;
if (BASE_B && process.env.PAIR_A) {
  const pairVia = async (page, base, code) => {
    await page.goto(base + '?tab=settings', { waitUntil: 'networkidle0' });
    await page.waitForSelector('details.alt');
    await page.$eval('details.alt', (e) => { e.open = true; });
    await page.waitForSelector('textarea');
    await page.$eval('textarea', (e, v) => { e.value = v; e.dispatchEvent(new Event('input', { bubbles: true })); }, code);
    await clickText(page, 'button', 'Pair with code');
    await page.waitForFunction(() => document.body.innerText.includes('Synced via'), { timeout: 20000 });
  };
  const a = await newPage(1000, 900);
  await pairVia(a, BASE, process.env.PAIR_A);
  // The pill re-probes after pairing, so give it a moment to leave "Device only" / "Checking…".
  const homed = await a.waitForFunction(() => document.querySelector('.pill .lbl')?.textContent?.trim() === 'Home', { timeout: 15000 }).then(() => true, () => false);
  ok(homed, 'pill shows "Home" when the LAN route verifies');
  await a.screenshot({ path: `${OUT}/settings-paired.png` });

  // Device A writes a note and syncs.
  await a.goto(BASE + '?tab=notes', { waitUntil: 'networkidle0' });
  await a.waitForSelector('textarea');
  await a.type('textarea', 'garrulous = loquacious = voluble');
  await clickText(a, 'button', 'Save note');
  await a.waitForFunction(() => document.querySelectorAll('.nt').length >= 1);
  await a.click('.pill');
  await a.waitForSelector('.pop');
  await a.screenshot({ path: `${OUT}/server-popover.png`, clip: { x: 500, y: 0, width: 500, height: 380 } });
  await clickText(a, '.pop button', 'Sync now');
  await a.waitForFunction(() => /Synced: sent/.test(document.querySelector('.pop')?.textContent ?? ''), { timeout: 20000 });
  ok(true, 'device A synced a note to the server');

  // Photograph notes: attach a photo, the server reads it, the person checks and saves it.
  await a.bringToFront();
  const photoPath = `${OUT}/handwriting.jpg`;
  execFileSync('convert', ['-size', '1000x300', 'xc:white', '-fill', 'black', '-pointsize', '54', '-annotate', '+40+130', 'mellifluous = dulcet', photoPath]);
  await a.goto(BASE + '?tab=notes', { waitUntil: 'networkidle0' });
  await a.waitForSelector('input[type=file][multiple]');
  await (await a.$('input[type=file][multiple]')).uploadFile(photoPath);
  await a.waitForSelector('.thumbs img');
  ok(true, 'a chosen photo shows as a thumbnail');
  await clickText(a, 'button', 'for reading');
  await a.waitForSelector('.draft');
  // The server reads it when we sync; keep syncing until the reading comes back.
  let ready = false;
  for (let n = 0; n < 30 && !ready; n++) {
    await a.evaluate(() => fetch('/api/sync_now', { method: 'POST', body: '{}' }));
    await a.goto(BASE + '?tab=notes', { waitUntil: 'networkidle0' });
    ready = (await a.$eval('.draft .st', (e) => e.textContent).catch(() => '')).includes('Ready to review');
    if (!ready) await new Promise((r) => setTimeout(r, 1000));
  }
  ok(ready, 'the server read the photo and it is ready to review');
  await a.click('.draft');
  await a.waitForSelector('.cols textarea');
  await a.waitForFunction(() => document.querySelector('.cols textarea').value.toLowerCase().includes('mellifluous'), { timeout: 8000 });
  ok(true, 'the machine reading appears as editable text');
  await a.waitForSelector('.photos img');
  ok(true, 'the photo is shown beside the text');
  await a.type('.cols textarea', '\nmellifluous = honeyed');
  await a.screenshot({ path: `${OUT}/draft-review.png`, fullPage: true });
  await clickText(a, 'button', 'Save as note');
  await a.waitForFunction(() => !document.querySelector('.cols textarea'), { timeout: 8000 });
  await a.goto(BASE + '?tab=notes', { waitUntil: 'networkidle0' });
  await a.waitForSelector('.nt');
  const became = await a.evaluate(() => [...document.querySelectorAll('.nt pre')].some((p) => p.textContent.includes('mellifluous = honeyed')));
  ok(became, 'the reviewed text became a note');
  await a.goto(BASE + '?tab=notes', { waitUntil: 'networkidle0' });
  ok(!(await a.$('.draft')), 'the draft left the review list');

  // The Claude tab is a general chat: a message goes out only when sent (the stand-in answers with an error, never spends quota).
  await a.goto(BASE + '?tab=claude', { waitUntil: 'networkidle0' });
  await clickText(a, 'button', 'New chat');
  await a.waitForSelector('.send textarea');
  ok(await a.$eval('.send textarea', (e) => { const r = e.getBoundingClientRect(); return r.top > 0 && r.bottom <= innerHeight; }), 'the message box is on screen, at the bottom');
  // The search suggestions open above the chat screen, not under it.
  await a.type('.search input, input[type=search], .search input[type=text]', 'abat');
  await a.waitForSelector('.dd li', { timeout: 8000 });
  ok(await a.$eval('.dd', (e) => { const r = e.getBoundingClientRect(); const t = document.elementFromPoint(r.left + r.width / 2, r.bottom - 6); return !!t && t.closest('.dd') !== null; }), 'search suggestions are above the chat screen');
  await a.$eval('.search input', (e) => { e.value = ''; e.dispatchEvent(new Event('input', { bubbles: true })); });
  ok(!(await a.$('.m')), 'the Claude chat is empty until the person writes');
  await a.type('.send textarea', 'The History tab feels slow');
  await clickText(a, '.send button', 'Send');
  await a.waitForSelector('.m.me');
  ok((await a.$eval('.m.me', (e) => e.textContent)).includes('History tab feels slow'), 'the message appears in the chat');
  await a.waitForSelector('.m.wait .spin', { timeout: 4000 });
  const t0 = await a.$eval('.m.wait .status', (e) => e.textContent);
  ok(/\d+s/.test(t0), 'the waiting message shows a running timer');
  ok(await a.$eval('.chead .btn', (e) => e.textContent.includes('Working')), 'the header says it is working, not a dead Delete button');
  await a.waitForFunction((x) => !document.querySelector('.m.wait .status') || document.querySelector('.m.wait .status').textContent !== x, { timeout: 3000 }, t0).catch(() => {});
  await a.waitForSelector('.m.bad', { timeout: 30000 });
  ok((await a.$eval('.m.bad', (e) => e.textContent)).includes('test stand-in'), 'a problem with Claude is shown in the chat, not hidden');
  ok(!(await a.$('.m.wait')), 'the waiting message is gone once the answer (or error) arrived');
  ok(await a.$eval('.chead .btn', (e) => e.textContent.includes('Delete')), 'a finished chat can be deleted');

  // Device B pairs and receives it, including its effect on the thesaurus.
  const b = await newPage(1000, 900);
  await pairVia(b, BASE_B, process.env.PAIR_B);
  await b.goto(BASE_B + '?tab=notes', { waitUntil: 'networkidle0' });
  await b.waitForSelector('.nt');
  await b.waitForFunction(() => [...document.querySelectorAll('.nt pre')].some((p) => p.textContent.includes('garrulous = loquacious = voluble')), { timeout: 10000 }).catch(() => {});
  ok(await b.evaluate(() => [...document.querySelectorAll('.nt pre')].some((p) => p.textContent.includes('garrulous = loquacious = voluble'))), "device B received device A's note");
  await b.goto(BASE_B + '?w=loquacious', { waitUntil: 'networkidle0' });
  await b.waitForSelector('.box.mine');
  const mineB = await b.$eval('.box.mine', (e) => e.textContent);
  ok(mineB.includes('garrulous') && mineB.includes('voluble'), "device B's thesaurus uses the synced note");
  await b.screenshot({ path: `${OUT}/thesaurus-device-b.png`, fullPage: true });

  // A lookup made on device B reaches device A's word arrows by the background sync alone (nobody presses Sync).
  await a.bringToFront();
  await a.goto(BASE + '?w=abate', { waitUntil: 'networkidle0' });
  await a.waitForSelector('.hw');
  await b.evaluate(() => fetch('/api/entry', { method: 'POST', body: '{"word":"venerate","source":"wiktionary"}' }).then(() => fetch('/api/sync_now', { method: 'POST', body: '{}' })));
  await a.evaluate(() => fetch('/api/sync_now', { method: 'POST', body: '{}' })); // stands in for A's background sync (no UI bump)
  const fwd = await a.waitForFunction(() => document.querySelector('.hn button.r span')?.textContent === 'venerate', { timeout: 10000 }).then(() => true, () => false);
  ok(fwd, "device B's lookup shows up as Forward on device A without pressing Sync");

  // Unpairing returns to device-only without losing data.
  await a.bringToFront(); // a background tab is throttled and would time out
  await a.goto(BASE + '?tab=settings', { waitUntil: 'networkidle0' });
  await clickText(a, 'button', 'Unpair');
  await a.waitForFunction(() => document.body.innerText.includes('Unpaired'));
  await a.goto(BASE + '?tab=notes', { waitUntil: 'networkidle0' });
  await a.waitForSelector('.nt');
  ok((await a.$$('.nt')).length >= 1, 'notes are kept after unpairing');
}

await browser.close();
console.log(failed ? `\n${failed} FAILED` : '\nall passed');
process.exit(failed ? 1 : 0);
