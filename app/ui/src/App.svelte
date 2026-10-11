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
    { id: 'search', label: 'Dictionary', icon: '📖' },
    { id: 'history', label: 'History', icon: '🕘' },
    { id: 'quiz', label: 'Quizzes', icon: '✅' },
    { id: 'notes', label: 'Notes', icon: '📝' },
    { id: 'settings', label: 'Settings', icon: '⚙️' },
  ];
</script>

<header class="top">
  <div class="wrap bar">
    <button class="brand" onclick={() => { app.tab = 'search'; app.word = ''; }} aria-label="Home">
      <span class="logo">L</span><span class="name">Lexis</span>
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
    <button class:on={app.tab === t.id} onclick={() => (app.tab = t.id)}>
      <span class="ic" aria-hidden="true">{t.icon}</span><span>{t.label}</span>
    </button>
  {/each}
</nav>
{/if}

<style>
  .top { background: var(--navy); color: #fff; position: sticky; top: 0; z-index: 20; padding-top: env(safe-area-inset-top); }
  .bar { display: flex; align-items: center; gap: 14px; padding-top: 10px; padding-bottom: 10px; }
  .brand { background: none; border: 0; color: #fff; display: flex; align-items: center; gap: 8px; padding: 0; }
  .logo {
    width: 34px; height: 34px; border-radius: 8px; background: var(--accent); display: grid; place-items: center;
    font-weight: 800; font-size: 20px;
  }
  .name { font-weight: 700; font-size: 20px; letter-spacing: 0.3px; }
  .search { flex: 1; min-width: 0; }
  .desk { display: flex; gap: 4px; padding-bottom: 0; }
  .desk button {
    background: none; border: 0; color: #cdd9ee; padding: 9px 14px; border-bottom: 3px solid transparent;
  }
  .desk button:hover { color: #fff; }
  .desk button.on { color: #fff; border-bottom-color: var(--accent); font-weight: 600; }
  .page { padding-top: 18px; padding-bottom: 96px; min-height: 70vh; }
  .tabbar { display: none; }
  @media (max-width: 720px) {
    .name { display: none; }
    .desk { display: none; }
    .tabbar {
      display: grid; grid-template-columns: repeat(5, 1fr); position: fixed; left: 0; right: 0; bottom: 0;
      background: var(--bg); border-top: 1px solid var(--line); z-index: 20;
      padding-bottom: env(safe-area-inset-bottom);
    }
    .tabbar button {
      background: none; border: 0; padding: 7px 2px 6px; display: flex; flex-direction: column; align-items: center;
      font-size: 11px; color: var(--muted); gap: 1px;
    }
    .tabbar button.on { color: var(--blue); font-weight: 600; }
    .ic { font-size: 20px; line-height: 1.1; }
  }
</style>
