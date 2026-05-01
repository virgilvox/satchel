<script lang="ts">
  import ResultRow from './ResultRow.svelte';
  import type { SearchResult } from '../lib/types';

  interface Props {
    results: SearchResult[];
    total: number;
    loading?: boolean;
    error?: string;
    canLoadMore?: boolean;
    onLoadMore?: () => void;
    onContext?: (r: SearchResult) => void;
    emptyText?: string;
  }
  let {
    results,
    total,
    loading = false,
    error,
    canLoadMore = false,
    onLoadMore,
    onContext,
    emptyText = 'no results found',
  }: Props = $props();
</script>

{#if loading}
  <div class="loading"><span class="glyph">::</span>searching...</div>
{:else if error}
  <div class="empty">{error}</div>
{:else if results.length === 0}
  <div class="empty"><span class="glyph">::</span>{emptyText}</div>
{:else}
  {#each results as r, i (i)}
    <ResultRow result={r} {onContext} />
  {/each}
  {#if canLoadMore}
    <div class="more-wrap">
      <span class="more-info">SHOWING {results.length} OF {total}</span>
      <button class="btn btn-secondary btn-sm" onclick={() => onLoadMore?.()}>LOAD MORE</button>
    </div>
  {:else if total > 0}
    <div class="more-wrap"><span class="more-info">ALL {total} RESULTS SHOWN</span></div>
  {/if}
{/if}

<style>
  .more-wrap {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 14px 0;
    gap: 12px;
  }
  .more-info {
    font-size: 10px;
    color: var(--text-dim);
    letter-spacing: 1.5px;
  }
</style>
