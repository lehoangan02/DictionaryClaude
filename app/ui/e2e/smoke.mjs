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
ok((await text(d, '.hw')) === 'abate' || (await text(d, '.hw')) === 'abat', 'Enter opens an entry');
await d.goto(BASE + '?w=abate', { waitUntil: 'networkidle0' });
await d.waitForSelector('.hw');
ok((await text(d, '.hw')) === 'abate', 'deep link opens the entry');
ok(!!(await d.$('.gre')), "Barron's card is shown for a GRE word");
ok((await d.$$('.senses > li')).length >= 2, 'several senses listed');
await clickText(d, '.srcs button', 'WordNet');
await d.waitForFunction(() => document.querySelector('.srcs button.on')?.textContent.includes('WordNet'));
await d.waitForFunction(() => !document.querySelector('article.dim'));
ok((await d.$$('.senses > li')).length >= 1, 'WordNet source shows senses');

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

// ---- two devices through a real sync server ----
const BASE_B = process.env.BASE_B;
if (BASE_B && process.env.PAIR_A) {
  const pairVia = async (page, base, code) => {
    await page.goto(base + '?tab=settings', { waitUntil: 'networkidle0' });
    await page.waitForSelector('textarea');
    await page.$eval('textarea', (e, v) => { e.value = v; e.dispatchEvent(new Event('input', { bubbles: true })); }, code);
    await clickText(page, 'button', 'Pair');
    await page.waitForFunction(() => document.body.innerText.includes('Synced via'), { timeout: 20000 });
  };
  const a = await newPage(1000, 900);
  await pairVia(a, BASE, process.env.PAIR_A);
  ok((await a.$eval('.pill .lbl', (e) => e.textContent)).trim() === 'Home', 'pill shows "Home" when the LAN route verifies');
  await a.screenshot({ path: `${OUT}/settings-paired.png` });

  // Device A writes a note and syncs.
  await a.goto(BASE + '?tab=notes', { waitUntil: 'networkidle0' });
  await a.waitForSelector('textarea');
  await a.type('textarea', 'garrulous = loquacious = voluble');
  await clickText(a, 'button', 'Save note');
  await a.waitForFunction(() => document.querySelectorAll('.nt').length >= 1);
  await a.click('.pill');
  await a.waitForSelector('.pop');
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
  await clickText(a, 'button', 'Ask agents');
  // Only check that the confirmation step exists; never press it (that spends quota).
  const buttons = await a.$$eval('button', (b) => b.map((x) => x.textContent.trim()));
  ok(buttons.some((t) => t.includes('use agent quota')), 'asking the agents needs a second confirmation (it costs quota)');
  await clickText(a, 'button', 'Cancel');
  ok((await a.$eval('body', (e) => e.innerText)).toLowerCase().includes('hard to read?'), 'the agent panel is idle until asked');
  await a.screenshot({ path: `${OUT}/draft-review.png`, fullPage: true });
  await clickText(a, 'button', 'Save as note');
  let became = true;
  await a.waitForFunction(() => [...document.querySelectorAll('.nt pre')].some((p) => p.textContent.includes('mellifluous = honeyed')), { timeout: 8000 }).catch(() => (became = false));
  ok(became, 'the reviewed text became a note');
  ok(!(await a.$('.draft')), 'the draft left the review list');

  // Device B pairs and receives it, including its effect on the thesaurus.
  const b = await newPage(1000, 900);
  await pairVia(b, BASE_B, process.env.PAIR_B);
  await b.goto(BASE_B + '?tab=notes', { waitUntil: 'networkidle0' });
  await b.waitForSelector('.nt');
  ok((await b.$eval('.nt pre', (e) => e.textContent)).includes('garrulous = loquacious = voluble'), "device B received device A's note");
  await b.goto(BASE_B + '?w=loquacious', { waitUntil: 'networkidle0' });
  await b.waitForSelector('.box.mine');
  const mineB = await b.$eval('.box.mine', (e) => e.textContent);
  ok(mineB.includes('garrulous') && mineB.includes('voluble'), "device B's thesaurus uses the synced note");
  await b.screenshot({ path: `${OUT}/thesaurus-device-b.png`, fullPage: true });

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
