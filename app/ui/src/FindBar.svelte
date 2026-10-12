<script lang="ts">
  import { onDestroy, tick } from 'svelte';
  import Icon from './Icon.svelte';

  // Find on page (Ctrl/Cmd+F), like a browser's: matches are painted with the CSS Custom Highlight API,
  // so the page itself is never edited and Svelte's own DOM stays untouched.
  let open = $state(false);
  let q = $state('');
  let count = $state(0);
  let cur = $state(0);
  let input = $state<HTMLInputElement>();
  let ranges: Range[] = [];
  let watch: MutationObserver | undefined;
  let later: ReturnType<typeof setTimeout>;

  const highlights: Map<string, unknown> | undefined = (CSS as unknown as { highlights?: Map<string, unknown> }).highlights;

  export const isOpen = () => open;

  export async function show() {
    const again = open; // Ctrl+F on an open bar just selects the text again, keeping the current match
    open = true;
    await tick();
    input?.focus();
    input?.select();
    scan(again);
    // Text arrives and leaves as screens change and entries load: keep the matches current while the bar is open.
    const root = document.querySelector('main');
    if (root && !watch) {
      watch = new MutationObserver(() => { clearTimeout(later); later = setTimeout(() => scan(true, false), 150); });
      watch.observe(root, { childList: true, subtree: true, characterData: true });
    }
  }

  export function hide() {
    open = false;
    watch?.disconnect(); watch = undefined;
    clearTimeout(later);
    ranges = []; count = 0;
    paint();
  }

  /** Collects every visible, case-insensitive match inside one text node (a match never spans two nodes). */
  function scan(keep: boolean, scroll = true) {
    ranges = [];
    const needle = q.toLowerCase();
    const root = document.querySelector('main');
    if (needle && root) {
      const walk = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
      for (let n = walk.nextNode(); n; n = walk.nextNode()) {
        const text = n as Text;
        const hay = text.data.toLowerCase();
        if (hay.length !== text.data.length) continue; // odd casing changes offsets: skip rather than mis-highlight
        for (let i = hay.indexOf(needle); i >= 0; i = hay.indexOf(needle, i + needle.length)) {
          const r = new Range();
          r.setStart(text, i); r.setEnd(text, i + needle.length);
          if (r.getClientRects().length) ranges.push(r); // hidden text (closed sections) has no boxes
        }
      }
    }
    count = ranges.length;
    cur = count ? (keep ? Math.min(cur, count - 1) : 0) : 0;
    paint(scroll);
  }

  function paint(scroll = false) {
    const now = ranges[cur];
    if (highlights) {
      if (!ranges.length) { highlights.delete('find'); highlights.delete('find-cur'); }
      else {
        highlights.set('find', new Highlight(...ranges));
        const c = new Highlight(now); c.priority = 1; highlights.set('find-cur', c);
      }
    } else if (now) {
      const sel = getSelection();
      sel?.removeAllRanges(); sel?.addRange(now);
    }
    if (scroll && now) {
      const r = now.getBoundingClientRect();
      const top = (document.querySelector('header.top')?.getBoundingClientRect().bottom ?? 0) + 24;
      if (r.top < top || r.bottom > innerHeight - 24) scrollBy({ top: r.top - innerHeight / 3 });
    }
  }

  function step(d: number) {
    if (!count) return;
    cur = (cur + d + count) % count;
    paint(true);
  }

  function key(e: KeyboardEvent) {
    if (e.key === 'Enter') { e.preventDefault(); step(e.shiftKey ? -1 : 1); }
  }

  onDestroy(() => { watch?.disconnect(); highlights?.delete('find'); highlights?.delete('find-cur'); });
</script>

{#if open}
  <div class="wrap findbar fadein" role="search" aria-label="Find on page">
    <div class="box">
      <input
        bind:this={input} bind:value={q} oninput={() => scan(false)} onkeydown={key}
        placeholder="Find on page" aria-label="Find on page" autocomplete="off" autocapitalize="off" spellcheck="false"
      />
      <span class="n" aria-live="polite">{q ? (count ? `${cur + 1} of ${count}` : 'No matches') : ''}</span>
      <button onclick={() => step(-1)} disabled={!count} aria-label="Previous match"><Icon name="up" size={22} /></button>
      <button onclick={() => step(1)} disabled={!count} aria-label="Next match"><Icon name="down" size={22} /></button>
      <button onclick={hide} aria-label="Close find"><Icon name="close" size={20} /></button>
    </div>
  </div>
{/if}

<style>
  .findbar { display: flex; justify-content: flex-end; padding-top: 6px; padding-bottom: 8px; }
  .box { display: flex; align-items: center; gap: 2px; width: min(100%, 420px); background: var(--panel); border: 1px solid var(--line); border-radius: 999px; padding: 3px 6px 3px 6px; }
  .box:focus-within { background: var(--surface); border-color: var(--blue); }
  input { flex: 1; min-width: 0; border: 0; background: none; outline: none; padding: 6px 8px 6px 12px; font-size: 16px; color: var(--text); }
  .n { color: var(--muted); font-size: 13px; white-space: nowrap; padding: 0 6px; }
  button { width: 32px; height: 32px; border: 0; border-radius: 50%; background: none; color: var(--muted); display: grid; place-items: center; flex: none; }
  button:hover:not(:disabled) { background: var(--panel-2); color: var(--text); }
  button:disabled { opacity: 0.35; cursor: default; }
</style>
