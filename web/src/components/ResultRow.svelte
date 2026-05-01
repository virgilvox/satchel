<script lang="ts">
  import type { SearchResult } from '../lib/types';
  interface Props {
    result: SearchResult;
    onContext?: (r: SearchResult) => void;
    truncate?: number;
  }
  let { result, onContext, truncate }: Props = $props();
  let display = $derived((result.score * 100).toFixed(2));
  let text = $derived.by(() => {
    if (!truncate) return result.text;
    return result.text.length > truncate ? result.text.slice(0, truncate) + '…' : result.text;
  });
</script>

<div class="result">
  <div class="meta">
    <span class="source">{result.source}</span>
    <span class="actions">
      {#if onContext}
        <span class="ctx-link" role="button" tabindex="0"
          onclick={() => onContext?.(result)}
          onkeydown={(e) => e.key === 'Enter' && onContext?.(result)}
        >show context</span>
      {/if}
      <span class="score" title="Reciprocal Rank Fusion score (×100)">{display}</span>
    </span>
  </div>
  <div class="text">{text}</div>
</div>

<style>
  .result {
    background: var(--surface);
    border: 1px solid var(--border);
    padding: 14px 16px;
    margin-bottom: 10px;
    transition: 120ms ease;
  }
  .result:hover { border-color: var(--border-strong); }
  .meta {
    display: flex;
    justify-content: space-between;
    align-items: center;
    margin-bottom: 8px;
    font-size: 11px;
    color: var(--text-dim);
    gap: 14px;
    flex-wrap: wrap;
  }
  .source { color: var(--text); word-break: break-all; }
  .actions { display: flex; gap: 12px; align-items: center; }
  .score {
    color: var(--amber);
    font-weight: 700;
    letter-spacing: 1px;
    font-size: 11px;
  }
  .ctx-link {
    font-size: 10px;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 1.5px;
    cursor: pointer;
    border-bottom: 1px solid transparent;
  }
  .ctx-link:hover { color: var(--teal); border-bottom-color: var(--teal); }
  .text {
    font-size: 12px;
    line-height: 1.65;
    white-space: pre-wrap;
    max-height: 220px;
    overflow-y: auto;
    color: var(--text);
    padding-right: 6px;
  }
</style>
