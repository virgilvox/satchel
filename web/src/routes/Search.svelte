<script lang="ts">
  import SearchBox from '../components/SearchBox.svelte';
  import ResultList from '../components/ResultList.svelte';
  import ViewHead from '../components/ViewHead.svelte';
  import Modal from '../components/Modal.svelte';
  import { api } from '../lib/api';
  import type { SearchResult } from '../lib/types';

  let q = $state('');
  let results = $state<SearchResult[]>([]);
  let total = $state(0);
  let loading = $state(false);
  let error = $state<string | undefined>();
  let lastQuery = $state('');
  const PAGE = 20;

  // Context modal state
  let ctxOpen = $state(false);
  let ctxSource = $state('');
  let ctxRecords = $state<Array<{ text: string; title?: string }>>([]);
  let ctxMatchText = $state('');
  let ctxLoading = $state(false);
  let ctxError = $state<string | undefined>();

  async function run(query: string) {
    if (!query.trim()) return;
    loading = true;
    error = undefined;
    try {
      const r = await api.search(query, PAGE, 0);
      if (r.error) { error = r.error; results = []; total = 0; }
      else { results = r.results ?? []; total = r.total ?? 0; }
      lastQuery = query;
    } catch (e) {
      error = (e as Error).message;
    } finally {
      loading = false;
    }
  }

  async function more() {
    const r = await api.search(lastQuery, PAGE, results.length);
    if (!r.error) {
      results = [...results, ...(r.results ?? [])];
      total = r.total ?? total;
    }
  }

  async function showContext(result: SearchResult) {
    ctxOpen = true;
    ctxSource = result.source;
    ctxMatchText = result.text;
    ctxLoading = true;
    ctxError = undefined;
    ctxRecords = [];
    try {
      const r = await api.conversation(result.source, 2000, 0);
      if (r.error) ctxError = r.error;
      else ctxRecords = r.records ?? [];
    } catch (e) {
      ctxError = (e as Error).message;
    } finally {
      ctxLoading = false;
    }
  }
</script>

<ViewHead num="03" title={`SEARCH <span class="slash">/</span> HYBRID RETRIEVAL`}
  desc="Semantic embeddings fused with keyword FTS via Reciprocal Rank Fusion. Higher score = more on-topic." />

<SearchBox bind:value={q} placeholder="natural-language query..." onsubmit={run} />

<div class="results">
  <ResultList {results} {total} {loading} {error}
    canLoadMore={results.length < total}
    onLoadMore={more}
    onContext={showContext}
    emptyText="no matches yet · enter a query above" />
</div>

<Modal open={ctxOpen} title="CONVERSATION CONTEXT" wide onClose={() => (ctxOpen = false)}>
  <div class="ctx-path">{ctxSource}</div>
  <div class="ctx-list">
    {#if ctxLoading}
      <div class="loading"><span class="glyph">::</span>loading conversation...</div>
    {:else if ctxError}
      <div class="empty">{ctxError}</div>
    {:else if ctxRecords.length === 0}
      <div class="empty"><span class="glyph">::</span>no records at this source</div>
    {:else}
      {#each ctxRecords as rec, i (i)}
        <div class="ctx-msg" class:match={rec.text === ctxMatchText}>
          {#if rec.title}<span class="ctx-title">{rec.title}</span>{/if}
          {rec.text}
        </div>
      {/each}
    {/if}
  </div>
</Modal>

<style>
  .results { margin-top: 18px; }
  .ctx-path {
    padding: 10px 18px;
    font-size: 11px;
    color: var(--teal);
    border-bottom: 1px solid var(--border);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ctx-list {
    flex: 1;
    overflow-y: auto;
    padding: 6px 0;
  }
  .ctx-msg {
    padding: 10px 18px;
    font-size: 12px;
    line-height: 1.6;
    white-space: pre-wrap;
    border-bottom: 1px solid var(--border);
  }
  .ctx-msg.match {
    background: var(--amber-soft);
    border-left: 2px solid var(--amber);
  }
  .ctx-title {
    display: block;
    font-size: 9px;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 2px;
    margin-bottom: 4px;
  }
</style>
