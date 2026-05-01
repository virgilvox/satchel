<script lang="ts">
  import type { Snippet } from 'svelte';
  interface Props {
    tone?: 'amber' | 'teal' | 'danger' | 'neutral';
    children?: Snippet;
  }
  let { tone = 'neutral', children }: Props = $props();
</script>

<span class="pill {tone}">
  {#if children}{@render children()}{/if}
</span>

<style>
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 5px 11px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 2px;
    text-transform: uppercase;
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text-dim);
  }
  .amber {
    color: var(--amber);
    border-color: var(--amber-line);
    background: var(--amber-soft);
  }
  .teal {
    color: var(--teal);
    border-color: var(--teal);
    background: var(--teal-soft);
  }
  .danger {
    color: var(--danger);
    border-color: var(--danger);
    background: var(--danger-soft);
  }
</style>
