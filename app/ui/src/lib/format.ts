export const POS: Record<string, string> = {
  adj: 'adjective', adv: 'adverb', noun: 'noun', verb: 'verb', prep: 'preposition',
  conj: 'conjunction', intj: 'interjection', pron: 'pronoun', det: 'determiner',
  name: 'proper noun', num: 'numeral', phrase: 'phrase', prefix: 'prefix', suffix: 'suffix',
  article: 'article', particle: 'particle', contraction: 'contraction', abbrev: 'abbreviation',
};
export const posLabel = (p: string) => POS[p] ?? p;

export type Sound = { ipa?: string; audio?: string; tags?: string[] };

const accent = (s: Sound) => {
  const t = (s.tags ?? []).join(' ');
  if (/US|California|General-American|Midwest/.test(t)) return 'US';
  if (/UK|Received-Pronunciation|RP|Southern-England/.test(t)) return 'UK';
  return '';
};

/** One IPA + one audio per accent, preferring UK then US like a learner's dictionary. */
export function pronunciations(blocks: { sounds?: Sound[] }[]) {
  const out: { accent: string; ipa?: string; audio?: string }[] = [];
  for (const a of ['UK', 'US', '']) {
    const all = blocks.flatMap((b) => b.sounds ?? []).filter((s) => accent(s) === a);
    const ipa = all.find((s) => s.ipa)?.ipa;
    const audio = all.find((s) => s.audio)?.audio;
    if (ipa || audio) out.push({ accent: a, ipa, audio });
  }
  return out.slice(0, 2);
}

import { call, isDesktopApp, native } from './api';

let current: HTMLAudioElement | null = null;

/**
 * Plays the recording; falls back to the system voice. The desktop app plays
 * through the system audio output (its web view cannot play audio when
 * packaged); a browser uses an <audio> element and the browser's voice.
 */
export async function speak(word: string, audio?: string): Promise<'recording' | 'voice'> {
  current?.pause();
  if (isDesktopApp) {
    if (audio) {
      try { await native('play_audio', { url: audio }); return 'recording'; } catch { /* offline and not saved yet */ }
    }
    try { await native('speak_text', { word }); } catch { /* no voice installed */ }
    return 'voice';
  }
  if (audio) {
    try {
      const { data_url } = await call<{ data_url: string }>('audio_get', { url: audio });
      current = new Audio(data_url);
      await current.play();
      return 'recording';
    } catch { /* fall through to speech */ }
  }
  // Android: the system voice, through the app (the web view has none of its own).
  const voice = (window as unknown as { LadyBirdVoice?: { speak(w: string): void } }).LadyBirdVoice;
  if (voice) { voice.speak(word); return 'voice'; }
  if ('speechSynthesis' in window) {
    const u = new SpeechSynthesisUtterance(word);
    u.lang = 'en-US';
    speechSynthesis.cancel();
    speechSynthesis.speak(u);
  }
  return 'voice';
}

export const when = (ms: number) =>
  new Date(ms).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });

export const pct = (a: number, b: number) => (b ? Math.round((100 * a) / b) : 0);

export const REL = {
  same: { sym: '=', label: 'Same meaning' },
  similar: { sym: '≈', label: 'Roughly similar' },
  opposite: { sym: '><', label: 'Opposite' },
  different: { sym: '⍯', label: 'Different / often confused' },
} as const;

/** A reading passage as flowing text, keeping the book's line numbers as markers. */
export function passageParts(passage: string): { n?: number; text: string }[] {
  const out: { n?: number; text: string }[] = [];
  let carry = '';
  for (const raw of passage.split('\n')) {
    const m = raw.match(/^\((\d+)\)\s*(.*)$/);
    let text = (m ? m[2] : raw).trim();
    if (carry) { text = carry + text; carry = ''; }
    // A word hyphenated across lines is rejoined; a real compound keeps its hyphen.
    if (/[a-z]-$/.test(text)) { carry = text.slice(0, -1); text = ''; }
    if (m) out.push({ n: Number(m[1]), text: text ? text + ' ' : '' });
    else if (text) out.push({ text: text + ' ' });
    // the carried half attaches to the next line's first word
    if (carry && !text) out.push({ text: '' });
  }
  return out.filter((p) => p.n !== undefined || p.text);
}

/** Splits a sentence so the headword (and its usual endings) can be highlighted. */
export function highlight(text: string, word: string): { t: string; hit: boolean }[] {
  const w = word.trim();
  if (!w || !text) return [{ t: text, hit: false }];
  const esc = (x: string) => x.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const alts = [`${esc(w)}(?:s|es|ed|d|ing|ly|ness|ity|er|est|ment|ments)?`];
  const last = w.at(-1)!;
  if (w.endsWith('e')) alts.push(`${esc(w.slice(0, -1))}(?:ing|ed|ation|ations|ity)`);
  if (w.endsWith('y')) alts.push(`${esc(w.slice(0, -1))}(?:ies|ied|ily|iness)`);
  if (/[bdglmnprt]$/.test(w)) alts.push(`${esc(w)}${last}(?:ed|ing|er)`);
  let re: RegExp;
  try { re = new RegExp(`(?<![\\p{L}])(?:${alts.join('|')})(?![\\p{L}])`, 'giu'); } catch { return [{ t: text, hit: false }]; }
  const out: { t: string; hit: boolean }[] = [];
  let at = 0;
  for (const m of text.matchAll(re)) {
    if (m.index! > at) out.push({ t: text.slice(at, m.index), hit: false });
    out.push({ t: m[0], hit: true });
    at = m.index! + m[0].length;
  }
  if (at < text.length) out.push({ t: text.slice(at), hit: false });
  return out.length ? out : [{ t: text, hit: false }];
}
