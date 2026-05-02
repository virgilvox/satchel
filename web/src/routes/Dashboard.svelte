<script lang="ts">
  import { onMount } from 'svelte';
  import StatCard from '../components/StatCard.svelte';
  import SearchBox from '../components/SearchBox.svelte';
  import ResultList from '../components/ResultList.svelte';
  import ViewHead from '../components/ViewHead.svelte';
  import { api, type ReleaseInfo } from '../lib/api';
  import { status } from '../lib/stores.svelte';
  import type { SearchResult } from '../lib/types';

  // Release probe — populated on mount via /api/release. Server caches
  // for an hour so the call is cheap. We never block render on it.
  let release = $state<ReleaseInfo | null>(null);
  let releaseChecking = $state(false);
  async function loadRelease(refresh = false) {
    if (releaseChecking) return;
    releaseChecking = true;
    try {
      release = await api.release(refresh);
    } catch {
      // network error swallowed — banner just won't show
    } finally {
      releaseChecking = false;
    }
  }
  onMount(() => loadRelease(false));

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
  let vault = $derived(status.data?.vault);
  let siblings = $derived(vault?.siblings ?? []);
  let legacy = $derived(vault?.legacy_bases ?? []);
  // The two diagnostics we surface as a banner — siblings under the
  // same base that aren't active, plus other SATCHEL bases discovered
  // entirely outside the chosen one. Either is a strong "you may be
  // looking at the wrong vault" signal when the dashboard count is
  // smaller than expected.
  let unusedDataMb = $derived(
    [...siblings, ...legacy].reduce((acc, v) => acc + v.size_bytes, 0) / (1024 * 1024),
  );
  let hasUnusedData = $derived(unusedDataMb > 1); // ignore empty schema-only DBs

  let dimsLabel = $derived(
    status.data ? `${status.data.embedding_model} · ${stats?.dimensions ?? '—'}d` : 'model status'
  );

  function fmtMb(bytes: number) {
    return bytes < 1024 * 1024
      ? `${(bytes / 1024).toFixed(1)} KB`
      : bytes < 1024 * 1024 * 1024
        ? `${(bytes / (1024 * 1024)).toFixed(1)} MB`
        : `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
  }
</script>

<ViewHead num="01" title={`DASHBOARD <span class="slash">/</span> VAULT AT A GLANCE`}
  desc="Counts, quick search, and a status check for your portable knowledge corpus." />

{#if release?.update_available && release.release_url}
  <a class="update-banner" href={release.release_url} target="_blank" rel="noopener">
    <span class="up-label">UPDATE AVAILABLE</span>
    <span class="up-vers">v{release.current} → v{release.latest}</span>
    <span class="up-hint">view release notes ↗</span>
  </a>
{/if}

{#if vault?.path}
  <div class="vault-strip">
    <span class="vault-label">ACTIVE VAULT</span>
    <span class="vault-name">{vault.name ?? 'default'}</span>
    <span class="vault-path" title={vault.path}>{vault.path}</span>
    <button class="up-check" type="button" onclick={() => loadRelease(true)}
      disabled={releaseChecking || release?.disabled}
      title={release?.disabled
        ? 'Update checks disabled via SATCHEL_DISABLE_UPDATE_CHECK'
        : release?.error
          ? `Last check: ${release.error}`
          : release?.checked_at
            ? `Last checked ${release.checked_at}`
            : 'Check GitHub for a newer release'}>
      {#if releaseChecking}CHECKING…{:else if release?.disabled}UPDATES OFF{:else if release?.update_available}v{release.latest} READY{:else if release?.latest}UP TO DATE{:else}CHECK FOR UPDATES{/if}
    </button>
  </div>
{/if}

{#if hasUnusedData}
  <div class="unused-banner">
    <strong>Other SATCHEL data detected on disk.</strong>
    {#if siblings.length > 0}
      Sibling vaults under the same base:
      {#each siblings as v, i (v.path)}
        <code class="vp">{v.name}</code> ({v.size_human}){i < siblings.length - 1 ? ',' : ''}
      {/each}
      — switch with <code>satchel vault use {siblings[0].name}</code> in a terminal, then restart.
    {/if}
    {#if legacy.length > 0}
      {#if siblings.length > 0}<br />{/if}
      Legacy SATCHEL base{legacy.length === 1 ? '' : 's'}:
      {#each legacy as v, i (v.path)}
        <code class="vp" title={v.path}>{v.path}</code> ({v.size_human}){i < legacy.length - 1 ? ',' : ''}
      {/each}
      — restart with <code>--vault {legacy[0].path}</code> to use {legacy.length === 1 ? 'it' : 'them'} instead. Total elsewhere: {fmtMb(unusedDataMb * 1024 * 1024)}.
    {/if}
  </div>
{/if}

<div class="section-label">VAULT STATS</div>
<div class="grid">
  <StatCard label="DOCS" value={stats?.documents ?? '—'} meta="documents in vault" />
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

  .update-banner {
    display: flex; align-items: center; gap: 12px; flex-wrap: wrap;
    margin: 0 0 12px;
    padding: 10px 14px;
    border: 1px solid var(--teal);
    background: var(--teal-soft);
    color: var(--text);
    text-decoration: none;
    font-size: 11px;
    transition: 120ms ease;
  }
  .update-banner:hover { background: var(--teal-soft); border-color: var(--teal-deep, var(--teal)); }
  .update-banner .up-label {
    color: var(--teal); font-weight: 700;
    letter-spacing: 2px; text-transform: uppercase;
  }
  .update-banner .up-vers {
    color: var(--text-bright); font-weight: 700; letter-spacing: 1px;
    font-family: var(--font-mono, ui-monospace, Menlo, monospace);
  }
  .update-banner .up-hint { color: var(--teal); margin-left: auto; }

  .up-check {
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text-dim);
    cursor: pointer;
    padding: 4px 10px;
    font-family: inherit;
    font-size: 9px;
    letter-spacing: 1.5px;
    text-transform: uppercase;
    font-weight: 700;
    transition: 120ms ease;
    margin-left: auto;
  }
  .up-check:hover:not(:disabled) {
    color: var(--text-bright); border-color: var(--border-strong);
  }
  .up-check:disabled { opacity: 0.6; cursor: default; }

  .vault-strip {
    display: flex; align-items: center; gap: 12px; flex-wrap: wrap;
    margin: 0 0 16px;
    padding: 10px 14px;
    border: 1px solid var(--border);
    background: var(--surface);
    font-size: 11px;
  }
  .vault-strip .vault-label {
    color: var(--text-dim);
    letter-spacing: 2px; text-transform: uppercase; font-weight: 700;
  }
  .vault-strip .vault-name {
    color: var(--amber); font-weight: 700; letter-spacing: 1px;
  }
  .vault-strip .vault-path {
    color: var(--text); font-family: var(--font-mono, ui-monospace, Menlo, monospace);
    font-size: 10px; word-break: break-all; flex: 1;
  }

  .unused-banner {
    margin: 0 0 18px;
    padding: 12px 14px;
    border: 1px solid var(--amber-line, var(--amber));
    background: var(--amber-soft);
    color: var(--text);
    font-size: 12px;
    line-height: 1.6;
  }
  .unused-banner code {
    font-family: var(--font-mono, ui-monospace, Menlo, monospace);
    font-size: 11px;
    background: var(--surface);
    padding: 1px 5px;
    border: 1px solid var(--border);
  }
  .unused-banner code.vp { color: var(--amber); }
  .unused-banner strong { color: var(--amber); letter-spacing: 0.5px; }
</style>
