<script lang="ts">
  import './lib/tokens.css';

  import TopBar from './components/TopBar.svelte';
  import SideBar from './components/SideBar.svelte';

  import Dashboard from './routes/Dashboard.svelte';
  import Ask from './routes/Ask.svelte';
  import Chat from './routes/Chat.svelte';
  import Search from './routes/Search.svelte';
  import Documents from './routes/Documents.svelte';
  import Ingest from './routes/Ingest.svelte';
  import Manage from './routes/Manage.svelte';
  import Connect from './routes/Connect.svelte';

  import { router, status } from './lib/stores.svelte';
  import { api } from './lib/api';

  let activeJobs = $state(0);

  async function poll() {
    try {
      const d = await api.status();
      status.set(d, true);
    } catch {
      status.set(null, false);
    }
  }

  async function pollJobs() {
    try {
      const r = await api.jobs();
      activeJobs = (r.jobs ?? []).filter(
        (j) => j.status === 'running' || j.status === 'pending'
      ).length;
    } catch {}
  }

  $effect(() => {
    poll();
    pollJobs();
    const t1 = window.setInterval(poll, 30_000);
    const t2 = window.setInterval(pollJobs, 5_000);
    return () => {
      clearInterval(t1);
      clearInterval(t2);
    };
  });
</script>

<div class="app">
  <TopBar />
  <SideBar {activeJobs} />
  <main class="main">
    {#if router.tab === 'dashboard'}<Dashboard />
    {:else if router.tab === 'ask'}<Ask />
    {:else if router.tab === 'chat'}<Chat />
    {:else if router.tab === 'search'}<Search />
    {:else if router.tab === 'documents'}<Documents />
    {:else if router.tab === 'ingest'}<Ingest />
    {:else if router.tab === 'manage'}<Manage />
    {:else if router.tab === 'connect'}<Connect />
    {/if}
  </main>
</div>

<style>
  .app {
    display: grid;
    grid-template-rows: auto 1fr;
    grid-template-columns: 240px 1fr;
    grid-template-areas: 'topbar topbar' 'sidebar main';
    min-height: 100vh;
  }
  .main {
    grid-area: main;
    padding: 36px 36px 96px;
    max-width: 1100px;
    width: 100%;
  }
  @media (max-width: 880px) {
    .app { grid-template-columns: 64px 1fr; }
    .main { padding: 24px 18px 64px; }
  }
</style>
