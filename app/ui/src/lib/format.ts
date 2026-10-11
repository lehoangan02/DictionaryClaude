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

import { call } from './api';

let current: HTMLAudioElement | null = null;

/** Plays the recording (from the offline cache, or fetched once when online); falls back to speech synthesis. */
export async function speak(word: string, audio?: string) {
  current?.pause();
  if (audio) {
    try {
      const { data_url } = await call<{ data_url: string }>('audio_get', { url: audio });
      current = new Audio(data_url);
      await current.play();
      return;
    } catch {
      /* offline and not cached yet, or the host refused: speak it instead */
    }
  }
  if ('speechSynthesis' in window) {
    const u = new SpeechSynthesisUtterance(word);
    u.lang = 'en-US';
    speechSynthesis.cancel();
    speechSynthesis.speak(u);
  }
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
