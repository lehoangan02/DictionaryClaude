<script lang="ts">
  // Small inline icon set (24x24 outline/filled paths); no font or network needed.
  let { name, size = 22 }: { name: string; size?: number } = $props();
  const P: Record<string, string> = {
    book: 'M5 4h11a3 3 0 0 1 3 3v13H8a3 3 0 0 1-3-3V4zm3 2v11a1 1 0 0 0 1 1h8V7a1 1 0 0 0-1-1H8z',
    history: 'M13 3a9 9 0 1 0 8.5 11.9l-1.9-.6A7 7 0 1 1 13 5v3l4-4-4-4v3zm-1 5v5l4 2.4.8-1.3-3.3-2V8H12z',
    quiz: 'M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20zm-1.2 14.6-3.9-3.9 1.4-1.4 2.5 2.5 5-5 1.4 1.4-6.4 6.4z',
    notes: 'M4 17.3V20h2.7L18.4 8.3l-2.7-2.7L4 17.3zM20.7 6.3a1 1 0 0 0 0-1.4l-1.6-1.6a1 1 0 0 0-1.4 0l-1.3 1.3 3 3 1.3-1.3z',
    menu: 'M3 6h18v2H3zm0 5h18v2H3zm0 5h18v2H3z',
    chat: 'M20 2H4a2 2 0 0 0-2 2v18l4-4h14a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2zM7 9h10v2H7zm0 4h7v2H7zm0-6h10v2H7z',
    settings: 'M19.4 13a7.5 7.5 0 0 0 0-2l2-1.6-2-3.4-2.4 1a7.4 7.4 0 0 0-1.7-1L15 3.5h-4l-.4 2.5a7.4 7.4 0 0 0-1.7 1l-2.4-1-2 3.4 2 1.6a7.5 7.5 0 0 0 0 2l-2 1.6 2 3.4 2.4-1a7.4 7.4 0 0 0 1.7 1l.4 2.5h4l.4-2.5a7.4 7.4 0 0 0 1.7-1l2.4 1 2-3.4-2-1.6zM13 15.5a3.5 3.5 0 1 1 0-7 3.5 3.5 0 0 1 0 7z',
    search: 'M15.5 14h-.8l-.3-.3A6.5 6.5 0 1 0 14 15.5l.3.3v.8l5 5 1.5-1.5-5-5zm-6 0a4.5 4.5 0 1 1 0-9 4.5 4.5 0 0 1 0 9z',
    star: 'M12 2l2.9 6.3 6.9.7-5.2 4.6 1.5 6.8L12 16.9 5.9 20.4l1.5-6.8L2.2 9l6.9-.7L12 2z',
    back: 'M20 11H7.8l5.6-5.6L12 4l-8 8 8 8 1.4-1.4L7.8 13H20v-2z',
    prev: 'M6 5h2v14H6V5zm3.5 7L19 5v14l-9.5-7z',
    next: 'M16 5h2v14h-2V5zM5 19V5l9.5 7L5 19z',
    speaker: 'M3 9v6h4l5 5V4L7 9H3zm13.5 3A4.5 4.5 0 0 0 14 8v8a4.5 4.5 0 0 0 2.5-4zM14 3.2v2.1a7 7 0 0 1 0 13.4v2.1a9 9 0 0 0 0-17.6z',
    close: 'M19 6.4 17.6 5 12 10.6 6.4 5 5 6.4 10.6 12 5 17.6 6.4 19 12 13.4 17.6 19 19 17.6 13.4 12 19 6.4z',
    plus: 'M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z',
    minus: 'M19 13H5v-2h14v2z',
    up: 'M7 14l5-5 5 5H7z',
    down: 'M7 10l5 5 5-5H7z',
    camera: 'M9 3 7.2 5H4a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2h-3.2L15 3H9zm3 15a5 5 0 1 1 0-10 5 5 0 0 1 0 10zm0-2a3 3 0 1 0 0-6 3 3 0 0 0 0 6z',
    image: 'M21 19V5a2 2 0 0 0-2-2H5a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2zM8.5 13.5l2.5 3 3.5-4.5 4.5 6H5l3.5-4.5z',
  };
</script>

<svg class="ic" width={size} height={size} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true"><path d={P[name] ?? ''} /></svg>
