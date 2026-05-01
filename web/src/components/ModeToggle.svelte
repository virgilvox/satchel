<script lang="ts">
  import { theme } from '../lib/stores';
</script>

<button class="mode-toggle" type="button" title="Toggle theme" onclick={() => theme.toggle()}>
  <span class="icon" aria-hidden="true"></span>
  <span>{theme.mode.toUpperCase()}</span>
</button>

<style>
  .mode-toggle {
    font-family: inherit;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 2px;
    text-transform: uppercase;
    padding: 6px 12px;
    background: var(--surface);
    color: var(--text-dim);
    border: 1px solid var(--border);
    cursor: pointer;
    display: flex;
    align-items: center;
    gap: 8px;
    transition: 120ms ease;
  }
  .mode-toggle:hover {
    color: var(--amber);
    border-color: var(--amber-line);
  }
  .icon {
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--text-bright);
  }
  :global(:root[data-mode='dark']) .icon { box-shadow: inset -3px 1px 0 var(--bg); }
  :global(:root[data-mode='light']) .icon { background: var(--amber); }
</style>
