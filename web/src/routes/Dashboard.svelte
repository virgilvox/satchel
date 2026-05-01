<script lang="ts">
  import StatCard from '../components/StatCard.svelte';
  import SearchBox from '../components/SearchBox.svelte';
  import ResultList from '../components/ResultList.svelte';
  import ViewHead from '../components/ViewHead.svelte';
  import { api } from '../lib/api';
  import { status } from '../lib/stores.svelte';
  import type { SearchResult } from '../lib/types';

  let q = $state('');
  let results = $state<SearchResult[]>([]);
  let total = $state(0);
  let loading = $state(false);
  let error = $state<string | undefined>();
  let lastQuery = $state('');
  const PAGE = 8;

  async function run(query: string) {
    if (!query.trim()) return;
    loading = true;
    error = undefined;
    try {
      const r = await api.search(query, PAGE, 0);
      if (r.error) {
        error = r.error;
        results = [];
        total = 0;
      } else {
        results = r.results ?? [];
        total = r.total ?? 0;
      }
      lastQuery = query;
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  }

  async function more() {
    try {
      const r = await api.search(lastQuery, PAGE, results.length);
      if (!r.error) {
        results = [...results, ...(r.results ?? [])];
        total = r.total ?? total;
      }
    } catch {}
  }

  let stats = $derived(status.data?.stats);
  let embedReady = $derived(status.data?.embedding_available ?? false);
  let canMore = $derived(results.length < total);

  let dimsLabel = $derived(
    status.data ? `${status.data.embedding_model} · ${stats?.dimensions ?? '—'}d` : 'model status'
  );
</script>

<ViewHead num="01" title={`DASHBOARD <span class="slash">/</span> VAULT AT A GLANCE`}
  desc="Counts, quick search, and a status check for your portable knowledge corpus." />

<div class="section-label">VAULT STATS</div>
<div class="grid">
  <StatCard label="DOCS" value={stats?.documents ?? '—'} meta="indexed source paths" />
  <StatCard label="CHUNKS" value={stats?.chunks ?? '—'} meta="embedded passages" />
  <StatCard label="DB SIZE" value={stats?.db_size ?? '—'} meta="on-disk footprint" />
  <StatCard
    label="EMBEDDINGS"
    value={embedReady ? 'READY' : 'OFFLINE'}
    meta={embedReady ? dimsLabel : 'run download-model.sh'}
    accent={embedReady}
  />
</div>

<div class="section-label">QUICK SEARCH</div>
<SearchBox bind:value={q} placeholder="search the vault..." onsubmit={run} />

<div class="results">
  {#if results.length || loading || error}
    <ResultList {results} {total} {loading} {error} canLoadMore={canMore} onLoadMore={more} />
  {/if}
</div>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
    gap: 14px;
    margin-bottom: 32px;
  }
  .results {
    margin-top: 14px;
  }
</style>
