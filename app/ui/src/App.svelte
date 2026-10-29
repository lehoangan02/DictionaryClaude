<script lang="ts">
  import { onMount } from 'svelte';
  import { app, boot, goBack, goForward, goTab, openMenu, closeMenu, menuToSettings, closeSettings, trackLocation, listenForBack, keepDownloadsGoing, watchHistory, type Tab } from './lib/store.svelte';
  import SearchBar from './SearchBar.svelte';
  import EntryView from './EntryView.svelte';
  import History from './History.svelte';
  import Quiz from './Quiz.svelte';
  import Notes from './Notes.svelte';
  import ThesaurusSearch from './ThesaurusSearch.svelte';
  import Forms from './Forms.svelte';
  import Settings from './Settings.svelte';
  import Claude from './Claude.svelte';
  import ServerPill from './ServerPill.svelte';
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';
  import SetupNotice from './SetupNotice.svelte';
  import Update from './Update.svelte';
  import FindBar from './FindBar.svelte';
  import { call } from './lib/api';
  import { bump } from './lib/store.svelte';
  import { enter, softFade } from './lib/motion';
  import { fly } from 'svelte/transition';

  // The bar's shadow deepens once content scrolls beneath it.
  let scrolled = $state(false);

  let ready = $state(false);
  let error = $state('');
  let find: FindBar;

  // Dictionaries can arrive on their own (the background setup): pick them up.
  async function refreshSources() {
    try {
      const next = await call('sources');
      if (JSON.stringify(next) !== JSON.stringify(app.sources)) { app.sources = next; bump(); }
    } catch { /* busy */ }
  }

  onMount(async () => {
    const poll = setInterval(refreshSources, 4000);
    try {
      await boot();
      ready = true;
      listenForBack();
      keepDownloadsGoing();
      watchHistory();
    } catch (e) {
      error = (e as Error).message;
    }
  });

  $effect(() => { void app.tab; void app.word; if (ready) trackLocation(); });

  const tabs: { id: Tab; label: string; icon: string }[] = [
    { id: 'search', label: 'Dictionary', icon: 'book' },
    { id: 'history', label: 'History', icon: 'history' },
    { id: 'quiz', label: 'Quizzes', icon: 'quiz' },
    { id: 'notes', label: 'Notes', icon: 'notes' },
    { id: 'thesaurus', label: 'Thesaurus', icon: 'swap' },
    { id: 'forms', label: 'Forms', icon: 'grid' },
    { id: 'claude', label: 'Claude', icon: 'chat' },
  ];

  // Keys that are text editing, or have their own meaning inside a control, are left alone.
  function typing(t: EventTarget | null) {
    if (!(t instanceof HTMLElement)) return false;
    if (t.isContentEditable || t.tagName === 'TEXTAREA' || t.tagName === 'SELECT') return true;
    return t instanceof HTMLInputElement && !['checkbox', 'button', 'submit', 'reset', 'image', 'file'].includes(t.type);
  }

  // Browser-style shortcuts. Up and Down scroll the page natively; they are left alone, never taken.
  function onKey(e: KeyboardEvent) {
    if (e.defaultPrevented || e.isComposing) return;
    if ((e.ctrlKey || e.metaKey) && !e.altKey && !e.shiftKey) {
      const k = e.key.toLowerCase();
      if (k === 'l') { // the "address bar": jump to the search box with its text selected
        e.preventDefault();
        const box = document.getElementById('lexis-q') as HTMLInputElement | null;
        box?.focus(); box?.select();
      } else if (k === 'f') { e.preventDefault(); find?.show(); }
      return;
    }
    if (e.key === 'Escape' && find?.isOpen() && (e.target as HTMLElement | null)?.id !== 'lexis-q') { find.hide(); return; }
    if (e.ctrlKey || e.metaKey || e.altKey || e.shiftKey || e.repeat || typing(e.target)) return;
    // Left and Right walk the words looked up, like a browser's Back and Forward.
    if ((e.key === 'ArrowLeft' || e.key === 'ArrowRight') && app.tab === 'search' && !app.focus) {
      e.preventDefault();
      if (e.key === 'ArrowLeft') goBack(); else goForward();
    }
  }
</script>

<svelte:window onscroll={() => (scrolled = window.scrollY > 4)} onkeydown={onKey} />

<header class="top" class:scrolled>
  <div class="wrap bar">
    <button class="brand" onclick={openMenu} aria-label="Menu">
      <Logo size={36} /><span class="name">LadyBird</span>
    </button>
    <div class="search"><SearchBar /></div>
    <ServerPill />
  </div>
  <nav class="wrap desk" aria-label="Sections">
    {#each tabs as t (t.id)}
      <button class:on={app.tab === t.id} onclick={() => goTab(t.id)}>{t.label}</button>
    {/each}
  </nav>
  <FindBar bind:this={find} />
</header>

{#if app.menu}
  <div class="scrim" onclick={closeMenu} role="presentation"></div>
  <aside class="drawer" aria-label="Menu" in:fly={{ x: -340, duration: 240 }} out:fly={{ x: -340, duration: 180 }}>
    <div class="dhead">LadyBird</div>
    <button class="item" onclick={menuToSettings}><Icon name="settings" size={24} /><span>Settings</span></button>
  </aside>
{/if}

{#if app.settings}
  <div class="scrim" onclick={closeSettings} role="presentation"></div>
  <aside class="panel" aria-label="Settings" in:fly={{ x: 420, duration: 240 }} out:fly={{ x: 420, duration: 180 }}>
    <div class="phead"><b>Settings</b><button class="gear" onclick={closeSettings} aria-label="Close settings"><Icon name="close" size={24} /></button></div>
    <div class="pbody"><Settings /></div>
  </aside>
{/if}

<main class="wrap page">
  {#if error}
    <p class="err">Couldn't start: {error}</p>
  {:else if !ready}
    <div class="skel" style="height:44px;width:60%;margin:30px auto"></div>
    <div class="skel" style="height:18px;width:80%;margin:12px auto"></div>
  {:else}
    <Update banner />
    <!-- Each screen slides up into place when you switch to it. -->
    {#key app.tab}
      <div class="view" in:enter={{ y: 14, duration: 260 }}>
        {#if app.tab === 'search' && app.sources.length === 0}
          <SetupNotice />
        {:else if app.tab === 'search'}
          <!-- Merriam-Webster works on its own over the internet; the offline dictionaries are a download. -->
          {#if !app.sources.some((x) => x.id !== 'mw')}
            <SetupNotice />
          {/if}
          <EntryView />
        {:else if app.tab === 'history'}
          <History />
        {:else if app.tab === 'quiz'}
          <Quiz />
        {:else if app.tab === 'notes'}
          <Notes />
        {:else if app.tab === 'thesaurus'}
          <ThesaurusSearch />
        {:else if app.tab === 'forms'}
          <Forms />
        {:else}
          <Claude />
        {/if}
      </div>
    {/key}
  {/if}
</main>

{#if !app.focus}
<nav class="tabbar" aria-label="Sections" in:enter={{ y: 40, duration: 260 }} out:softFade>
  {#each tabs as t (t.id)}
    <button class:on={app.tab === t.id} onclick={() => goTab(t.id)} aria-label={t.label}>
      <Icon name={t.icon} size={24} /><span>{t.label}</span>
    </button>
  {/each}
</nav>
{/if}

<style>
  .gear { background: none; border: 0; color: var(--muted); width: 40px; height: 40px; border-radius: 50%; display: grid; place-items: center; flex: none; }
  .gear:hover { background: var(--panel-2); color: var(--text); }
  .scrim { position: fixed; inset: 0; z-index: 40; background: rgba(10, 20, 40, 0.38); }
  .drawer {
    position: fixed; z-index: 41; top: 0; left: 0; bottom: 0; width: min(340px, 86%);
    background: var(--surface); box-shadow: 10px 0 40px rgba(16, 40, 80, 0.25); display: flex; flex-direction: column;
    padding: calc(24px + env(safe-area-inset-top)) 12px 12px; border-radius: 0 24px 24px 0;
  }
  .dhead { font-size: 30px; font-weight: 700; padding: 8px 12px 22px; }
  .item { display: flex; align-items: center; gap: 16px; background: none; border: 0; border-radius: 999px; padding: 14px 16px; font-size: 18px; color: var(--text); text-align: left; }
  .item:hover, .item:active { background: var(--panel-2); }
  .panel {
    position: fixed; z-index: 41; top: 0; right: 0; bottom: 0; width: min(520px, 100%);
    background: var(--bg); box-shadow: -10px 0 40px rgba(16, 40, 80, 0.25); display: flex; flex-direction: column;
    padding-top: env(safe-area-inset-top);
  }
  .phead { display: flex; align-items: center; justify-content: space-between; padding: 8px 12px 8px 20px; border-bottom: 1px solid var(--line); background: var(--surface); font-size: 18px; }
  .pbody { overflow-y: auto; padding: 4px 16px calc(24px + env(safe-area-inset-bottom)); flex: 1; -webkit-overflow-scrolling: touch; }
  /* A floating bar: white, lifted off the page by its shadow, it stays put while content scrolls. */
  .top {
    position: sticky; top: 0; z-index: 20; padding-top: env(safe-area-inset-top);
    background: color-mix(in srgb, var(--surface) 94%, transparent);
    backdrop-filter: blur(10px); -webkit-backdrop-filter: blur(10px);
    box-shadow: 0 1px 0 var(--line), 0 6px 18px rgba(16, 40, 80, 0.08);
    transition: box-shadow var(--t) var(--ease);
  }
  .top.scrolled { box-shadow: 0 1px 0 var(--line), 0 10px 28px rgba(16, 40, 80, 0.16); }
  .bar { display: flex; align-items: center; gap: 14px; padding-top: 10px; padding-bottom: 8px; }
  .brand { background: none; border: 0; color: var(--blue); display: flex; align-items: center; gap: 8px; padding: 4px 12px 4px 4px; margin-left: -4px; border-radius: 12px; flex: none; transition: background 0.12s; }
  .brand:hover, .brand:active { background: var(--panel-2); }
  /* The logo's corners are 22% of its size (8px at 36px); the hover shape is that plus the 4px padding, so the two curves stay concentric. */
  .brand :global(img) { box-shadow: none !important; }
  .name { font-weight: 700; font-size: 22px; letter-spacing: 0.2px; }
  .search { flex: 1; min-width: 0; }
  .desk { display: flex; gap: 2px; padding-bottom: 0; }
  .desk button { position: relative; background: none; border: 0; color: var(--muted); padding: 9px 16px; font-weight: 500; }
  .desk button::after { content: ''; position: absolute; left: 12px; right: 12px; bottom: 0; height: 3px; border-radius: 3px 3px 0 0; background: var(--blue); transform: scaleX(0); transition: transform var(--t) var(--ease); }
  .desk button:hover { color: var(--blue); }
  .desk button.on { color: var(--blue); font-weight: 700; }
  .desk button.on::after { transform: scaleX(1); }
  .page { padding-top: 20px; padding-bottom: 110px; min-height: 70vh; }
  .tabbar { display: none; }
  @media (max-width: 720px) {
    .name { display: none; }
    .desk { display: none; }
    /* Floating pill navigation at the bottom. */
    .tabbar {
      display: grid; grid-template-columns: repeat(7, 1fr); position: fixed; z-index: 20;
      left: 12px; right: 12px; bottom: calc(10px + env(safe-area-inset-bottom));
      background: var(--surface); border: 1px solid var(--line); border-radius: 24px; box-shadow: var(--shadow); padding: 4px;
    }
    /* Content scrolls under the pill: fade it out so no text shows in the gaps around it. */
    .tabbar::before {
      content: ''; position: fixed; z-index: -1; left: 0; right: 0; bottom: 0; pointer-events: none;
      height: calc(78px + env(safe-area-inset-bottom)); background: linear-gradient(to bottom, transparent, var(--bg) 45%);
    }
    .tabbar button { background: none; border: 0; padding: 7px 2px 6px; display: flex; flex-direction: column; align-items: center; font-size: 10px; color: var(--muted); gap: 1px; border-radius: 18px; min-width: 0; }
    .tabbar button { transition: background-color var(--t) var(--ease), color var(--t) var(--ease), transform var(--t-fast) var(--ease); }
    .tabbar button.on { color: var(--blue); background: var(--blue-soft); font-weight: 700; }
  }
</style>
