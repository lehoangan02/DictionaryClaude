// A small, safe Markdown subset for Claude's replies: paragraphs, headings, lists, fenced code,
// `inline code`, **bold**, *italic*. Everything is HTML-escaped first, so no markup from the reply survives.
const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

function inline(s: string): string {
  return esc(s)
    .replace(/`([^`\n]+)`/g, '<code>$1</code>')
    .replace(/\*\*([^*\n]+)\*\*/g, '<strong>$1</strong>')
    .replace(/(^|[\s(])\*([^*\n]+)\*(?=[\s).,;:!?]|$)/g, '$1<em>$2</em>');
}

export function markdown(src: string): string {
  const out: string[] = [];
  const lines = src.replace(/\r\n/g, '\n').split('\n');
  let i = 0;
  while (i < lines.length) {
    const l = lines[i];
    const fence = l.match(/^\s*```\s*([\w+-]*)\s*$/);
    if (fence) {
      const code: string[] = [];
      for (i++; i < lines.length && !/^\s*```\s*$/.test(lines[i]); i++) code.push(lines[i]);
      i++;
      out.push(`<pre class="code"${fence[1] ? ` data-lang="${esc(fence[1])}"` : ''}><code>${esc(code.join('\n'))}</code></pre>`);
      continue;
    }
    const h = l.match(/^(#{1,4})\s+(.*)$/);
    if (h) { out.push(`<p class="h"><strong>${inline(h[2])}</strong></p>`); i++; continue; }
    const li = l.match(/^\s*([-*]|\d+\.)\s+(.*)$/);
    if (li) {
      const ordered = /\d/.test(li[1]);
      const items: string[] = [];
      for (; i < lines.length; i++) {
        const m = lines[i].match(/^\s*([-*]|\d+\.)\s+(.*)$/);
        if (!m || /\d/.test(m[1]) !== ordered) break;
        items.push(`<li>${inline(m[2])}</li>`);
      }
      out.push(ordered ? `<ol>${items.join('')}</ol>` : `<ul>${items.join('')}</ul>`);
      continue;
    }
    if (!l.trim()) { i++; continue; }
    const para: string[] = [];
    for (; i < lines.length && lines[i].trim() && !/^\s*(```|#{1,4}\s|([-*]|\d+\.)\s)/.test(lines[i]); i++) para.push(inline(lines[i]));
    out.push(`<p>${para.join('<br>')}</p>`);
  }
  return out.join('');
}

/** `git diff --stat` text as rows: file, changed lines, and the +/- split. */
export function diffRows(stat: string): { file: string; n: number; add: number; del: number }[] {
  const rows = [];
  for (const l of stat.split('\n')) {
    const m = l.match(/^\s*(.+?)\s+\|\s+(\d+)\s*([+-]*)\s*$/);
    if (!m) continue;
    const plus = (m[3].match(/\+/g) ?? []).length, minus = (m[3].match(/-/g) ?? []).length;
    const tot = plus + minus || 1, n = +m[2];
    rows.push({ file: m[1], n, add: Math.round((plus / tot) * n), del: Math.round((minus / tot) * n) });
  }
  return rows;
}
