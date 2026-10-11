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

<header class="top">
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
    <p class="empty">Loading…</p>
  {:else if app.tab === 'search'}
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
</main>

{#if !app.focus}
<nav class="tabbar" aria-label="Sections">
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
  }
  .bar { display: flex; align-items: center; gap: 14px; padding-top: 10px; padding-bottom: 8px; }
  .brand { background: none; border: 0; color: var(--blue); display: flex; align-items: center; gap: 8px; padding: 0; }
  .name { font-weight: 700; font-size: 22px; letter-spacing: 0.2px; }
  .search { flex: 1; min-width: 0; }
  .desk { display: flex; gap: 2px; padding-bottom: 0; }
  .desk button { background: none; border: 0; color: var(--muted); padding: 9px 16px; border-bottom: 3px solid transparent; font-weight: 500; }
  .desk button:hover { color: var(--blue); }
  .desk button.on { color: var(--blue); border-bottom-color: var(--blue); font-weight: 700; }
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
    .tabbar button { background: none; border: 0; padding: 7px 2px 6px; display: flex; flex-direction: column; align-items: center; font-size: 11px; color: var(--muted); gap: 1px; border-radius: 18px; }
    .tabbar button.on { color: var(--blue); background: var(--blue-soft); font-weight: 700; }
  }
</style>
