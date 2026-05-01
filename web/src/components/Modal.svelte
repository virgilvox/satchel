<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    open: boolean;
    title: string;
    wide?: boolean;
    onClose: () => void;
    children?: Snippet;
    footer?: Snippet;
  }
  let { open, title, wide = false, onClose, children, footer }: Props = $props();
</script>

{#if open}
  <div
    class="backdrop"
    role="dialog"
    aria-modal="true"
    onclick={(e) => e.target === e.currentTarget && onClose()}
    onkeydown={(e) => e.key === 'Escape' && onClose()}
    tabindex="-1"
  >
    <div class="modal" class:wide>
      <div class="head">
        <span class="title">{title}</span>
        <button class="close" type="button" onclick={onClose} aria-label="Close">&times;</button>
      </div>
      {#if children}{@render children()}{/if}
      {#if footer}<div class="foot">{@render footer()}</div>{/if}
    </div>
  </div>
{/if}

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.65);
    z-index: 100;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px;
  }
  .modal {
    background: var(--surface);
    border: 1px solid var(--border);
    width: min(720px, 100%);
    max-height: 80vh;
    display: flex;
    flex-direction: column;
    box-shadow: var(--shadow-frame);
  }
  .modal.wide {
    width: min(900px, 100%);
    max-height: 86vh;
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: 14px 18px;
    border-bottom: 1px solid var(--border);
  }
  .title {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 2.5px;
    text-transform: uppercase;
    color: var(--amber);
  }
  .close {
    background: transparent;
    border: none;
    color: var(--text-dim);
    font-size: 22px;
    cursor: pointer;
    line-height: 1;
  }
  .close:hover { color: var(--text-bright); }
  .foot {
    display: flex;
    gap: 10px;
    justify-content: flex-end;
    padding: 12px 18px;
    border-top: 1px solid var(--border);
  }
</style>
