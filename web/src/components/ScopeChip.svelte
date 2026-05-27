<script lang="ts">
  import { collection, router } from '../lib/stores.svelte';

  // The TopBar scope chip. Reads the global active collection and lets
  // the user switch from any page. Management (create / delete) lives
  // on the Documents tab so this dropdown stays a short, focused list.

  let open = $state(false);
  let rootEl: HTMLDivElement | undefined = $state();

  function close() {
    open = false;
  }

  function pick(id: number | null) {
    collection.setActive(id);
    close();
  }

  function gotoDocuments() {
    router.set('documents');
    close();
  }

  // Close on outside click. Using `mousedown` so the click that opens a
  // freshly mounted button does not immediately trigger this close.
  function onWindowMouseDown(e: MouseEvent) {
    if (!open) return;
    if (rootEl && !rootEl.contains(e.target as Node)) close();
  }

  function onWindowKey(e: KeyboardEvent) {
    if (e.key === 'Escape' && open) close();
  }

  $effect(() => {
    window.addEventListener('mousedown', onWindowMouseDown);
    window.addEventListener('keydown', onWindowKey);
    return () => {
      window.removeEventListener('mousedown', onWindowMouseDown);
      window.removeEventListener('keydown', onWindowKey);
    };
  });

  let activeLabel = $derived(collection.activeName ?? 'ALL');
  let activeIsAll = $derived(collection.activeId == null);
</script>

<div class="root" bind:this={rootEl}>
  <button
    type="button"
    class="chip"
    class:scoped={!activeIsAll}
    aria-haspopup="menu"
    aria-expanded={open}
    onclick={() => (open = !open)}
    title={activeIsAll ? 'Vault scope: ALL' : `Vault scope: ${activeLabel}`}
  >
    <span class="lbl">SCOPE</span>
    <span class="val">{activeLabel}</span>
    <span class="caret" aria-hidden="true">{open ? '▴' : '▾'}</span>
  </button>

  {#if open}
    <div class="menu" role="menu">
      <div class="menu-head">CHOOSE COLLECTION</div>
      <button
        type="button"
        class="item"
        class:active={activeIsAll}
        role="menuitemradio"
        aria-checked={activeIsAll}
        onclick={() => pick(null)}
      >
        <span class="dot" aria-hidden="true"></span>
        <span class="item-label">ALL</span>
        <span class="item-meta">whole vault</span>
      </button>

      {#if collection.list.length === 0}
        <div class="empty">
          No collections yet. Create one on the Documents tab.
        </div>
      {:else}
        {#each collection.list as c (c.id)}
          <button
            type="button"
            class="item"
            class:active={collection.activeId === c.id}
            role="menuitemradio"
            aria-checked={collection.activeId === c.id}
            onclick={() => pick(c.id)}
          >
            <span class="dot" aria-hidden="true"></span>
            <span class="item-label">{c.name}</span>
            <span class="item-meta">{c.document_count} docs</span>
          </button>
        {/each}
      {/if}

      <div class="menu-foot">
        <button type="button" class="link" onclick={gotoDocuments}>
          Manage on Documents tab
        </button>
      </div>
    </div>
  {/if}
</div>

<style>
  .root {
    position: relative;
  }

  .chip {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text);
    padding: 4px 10px;
    font-family: inherit;
    font-size: 10px;
    letter-spacing: 1.5px;
    text-transform: uppercase;
    font-weight: 700;
    cursor: pointer;
    transition: 120ms ease;
  }
  .chip:hover {
    border-color: var(--border-strong);
    color: var(--text-bright);
  }
  .chip.scoped {
    color: var(--amber);
    border-color: var(--amber);
    background: var(--amber-soft);
  }
  .lbl {
    color: var(--text-dim);
    font-weight: 700;
  }
  .chip.scoped .lbl {
    color: var(--amber);
  }
  .val {
    font-weight: 700;
  }
  .caret {
    color: var(--text-dim);
    font-weight: 400;
    font-size: 9px;
  }

  .menu {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 60;
    min-width: 240px;
    background: var(--bg);
    border: 1px solid var(--border);
    box-shadow: var(--shadow-frame);
    padding: 6px 0;
    display: flex;
    flex-direction: column;
  }
  .menu-head {
    padding: 8px 14px 6px;
    font-size: 9px;
    letter-spacing: 2px;
    color: var(--text-dim);
    text-transform: uppercase;
  }
  .item {
    display: grid;
    grid-template-columns: 10px 1fr auto;
    align-items: center;
    gap: 10px;
    padding: 8px 14px;
    background: transparent;
    border: 0;
    color: var(--text);
    text-align: left;
    cursor: pointer;
    font-family: inherit;
    font-size: 11px;
    letter-spacing: 1px;
    text-transform: uppercase;
  }
  .item:hover {
    background: var(--surface);
  }
  .item.active {
    color: var(--amber);
    background: var(--amber-soft);
  }
  .dot {
    width: 8px;
    height: 8px;
    border: 1px solid var(--border-strong);
    border-radius: 50%;
    background: transparent;
  }
  .item.active .dot {
    background: var(--amber);
    border-color: var(--amber);
  }
  .item-label {
    font-weight: 700;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .item-meta {
    font-size: 9px;
    color: var(--text-dim);
    letter-spacing: 1px;
    text-transform: none;
  }
  .item.active .item-meta {
    color: var(--amber);
  }
  .empty {
    padding: 10px 14px;
    color: var(--text-dim);
    font-size: 11px;
    letter-spacing: 0.5px;
    text-transform: none;
  }
  .menu-foot {
    border-top: 1px solid var(--border);
    margin-top: 4px;
    padding: 8px 14px;
  }
  .link {
    background: transparent;
    border: 0;
    padding: 0;
    color: var(--teal);
    font-family: inherit;
    font-size: 10px;
    letter-spacing: 1.5px;
    text-transform: uppercase;
    cursor: pointer;
  }
  .link:hover {
    color: var(--teal-deep);
  }
</style>
