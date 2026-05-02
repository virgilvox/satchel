<script lang="ts">
  import { onMount } from 'svelte';
  import { api, type CollectionSummary } from '../lib/api';

  // Inline scope chip used by the Search and Ask routes. The Documents
  // tab has its own tab strip and doesn't render this. `value` is the
  // numeric collection id, or `''` for "scope = the whole vault".
  interface Props {
    value: number | '';
    onchange?: (next: number | '') => void;
  }
  let { value = $bindable(''), onchange }: Props = $props();

  let collections = $state<CollectionSummary[]>([]);
  let loaded = $state(false);

  onMount(async () => {
    const r = await api.collectionsList();
    if (!r.error) collections = r.collections ?? [];
    loaded = true;
  });

  function pick(next: number | '') {
    value = next;
    onchange?.(next);
  }
</script>

{#if loaded && collections.length > 0}
  <div class="scope" role="radiogroup" aria-label="Search scope">
    <span class="lbl">scope</span>
    <button class="chip" type="button" role="radio"
      aria-checked={value === ''}
      class:active={value === ''}
      onclick={() => pick('')}>ALL</button>
    {#each collections as c (c.id)}
      <button class="chip" type="button" role="radio"
        aria-checked={value === c.id}
        class:active={value === c.id}
        onclick={() => pick(c.id)}>{c.name}</button>
    {/each}
  </div>
{/if}

<style>
  .scope {
    display: flex; gap: 6px; align-items: center; flex-wrap: wrap;
    margin: 10px 0 0;
  }
  .lbl {
    font-size: 9px; color: var(--text-dim);
    letter-spacing: 2px; text-transform: uppercase;
    margin-right: 4px;
  }
  .chip {
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text-dim);
    cursor: pointer;
    padding: 4px 10px;
    font-family: inherit;
    font-size: 10px;
    letter-spacing: 1.5px;
    text-transform: uppercase;
    font-weight: 700;
    transition: 120ms ease;
  }
  .chip:hover { color: var(--text-bright); border-color: var(--border-strong); }
  .chip.active {
    color: var(--amber);
    border-color: var(--amber);
    background: var(--amber-soft);
  }
</style>
