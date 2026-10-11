<script lang="ts">
  import { onMount } from 'svelte';
  import { app, boot, type Tab } from './lib/store.svelte';
  import SearchBar from './SearchBar.svelte';
  import EntryView from './EntryView.svelte';
  import History from './History.svelte';
  import Quiz from './Quiz.svelte';
  import Notes from './Notes.svelte';
  import Settings from './Settings.svelte';
  import ServerPill from './ServerPill.svelte';
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';
  import { enter, softFade } from './lib/motion';

  // The bar's shadow deepens once content scrolls beneath it.
  let scrolled = $state(false);

  let ready = $state(false);
  let error = $state('');

  onMount(async () => {
    try {
      await boot();
      ready = true;
    } catch (e) {
      error = (e as Error).message;
    }
  });

  const tabs: { id: Tab; label: string; icon: string }[] = [
    { id: 'search', label: 'Dictionary', icon: 'book' },
    { id: 'history', label: 'History', icon: 'history' },
    { id: 'quiz', label: 'Quizzes', icon: 'quiz' },
    { id: 'notes', label: 'Notes', icon: 'notes' },
    { id: 'settings', label: 'Settings', icon: 'settings' },
  ];
</script>

<svelte:window onscroll={() => (scrolled = window.scrollY > 4)} />

<header class="top" class:scrolled>
  <div class="wrap bar">
    <button class="brand" onclick={() => { app.tab = 'search'; app.word = ''; }} aria-label="Home">
      <Logo size={36} /><span class="name">LadyBird</span>
    </button>
    <div class="search"><SearchBar /></div>
    <ServerPill />
  </div>
  <nav class="wrap desk" aria-label="Sections">
    {#each tabs as t (t.id)}
      <button class:on={app.tab === t.id} onclick={() => (app.tab = t.id)}>{t.label}</button>
    {/each}
  </nav>
</header>

<main class="wrap page">
  {#if error}
    <p class="err">Couldn't start: {error}</p>
  {:else if !ready}
    <div class="skel" style="height:44px;width:60%;margin:30px auto"></div>
    <div class="skel" style="height:18px;width:80%;margin:12px auto"></div>
  {:else}
    <!-- Each screen slides up into place when you switch to it. -->
    {#key app.tab}
      <div class="view" in:enter={{ y: 14, duration: 260 }}>
        {#if app.tab === 'search' && app.sources.length === 0}
          <div class="card empty">
            <h2>Get your dictionaries</h2>
            <p class="muted">This device has no dictionaries yet. Pair it with your server and download them in Settings, which keeps the app itself small.</p>
            <button class="btn primary" onclick={() => (app.tab = 'settings')}>Open Settings</button>
          </div>
        {:else if app.tab === 'search'}
          <!-- Merriam-Webster works on its own over the internet; the offline dictionaries are a download. -->
          {#if !app.sources.some((x) => x.id !== 'mw')}
            <div class="card nodict">
              <span class="muted">Looking words up in Merriam-Webster over the internet. Download the offline dictionaries in Settings.</span>
              <button class="btn small" onclick={() => (app.tab = 'settings')}>Settings</button>
            </div>
          {/if}
          <EntryView />
        {:else if app.tab === 'history'}
          <History />
        {:else if app.tab === 'quiz'}
          <Quiz />
        {:else if app.tab === 'notes'}
          <Notes />
        {:else}
          <Settings />
        {/if}
      </div>
    {/key}
  {/if}
</main>

{#if !app.focus}
<nav class="tabbar" aria-label="Sections" in:enter={{ y: 40, duration: 260 }} out:softFade>
  {#each tabs as t (t.id)}
    <button class:on={app.tab === t.id} onclick={() => (app.tab = t.id)} aria-label={t.label}>
      <Icon name={t.icon} size={24} /><span>{t.label}</span>
    </button>
  {/each}
</nav>
{/if}

<style>
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
  .brand { background: none; border: 0; color: var(--blue); display: flex; align-items: center; gap: 8px; padding: 0; }
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
      display: grid; grid-template-columns: repeat(5, 1fr); position: fixed; z-index: 20;
      left: 12px; right: 12px; bottom: calc(10px + env(safe-area-inset-bottom));
      background: var(--surface); border: 1px solid var(--line); border-radius: 24px; box-shadow: var(--shadow); padding: 4px;
    }
    /* Content scrolls under the pill: fade it out so no text shows in the gaps around it. */
    .tabbar::before {
      content: ''; position: fixed; z-index: -1; left: 0; right: 0; bottom: 0; pointer-events: none;
      height: calc(78px + env(safe-area-inset-bottom)); background: linear-gradient(to bottom, transparent, var(--bg) 45%);
    }
    .tabbar button { background: none; border: 0; padding: 7px 2px 6px; display: flex; flex-direction: column; align-items: center; font-size: 11px; color: var(--muted); gap: 1px; border-radius: 18px; }
    .tabbar button { transition: background-color var(--t) var(--ease), color var(--t) var(--ease), transform var(--t-fast) var(--ease); }
    .tabbar button.on { color: var(--blue); background: var(--blue-soft); font-weight: 700; }
  }
.nodict { display: flex; align-items: center; gap: 12px; justify-content: space-between; margin: 0 0 14px; padding: 10px 14px; font-size: 14px; }
.empty { margin: 24px 0; display: grid; gap: 12px; justify-items: start; text-align: left; } .empty :global(h2), .empty :global(p) { margin: 0; }
</style>
