<script lang="ts">
  import Mark from './Mark.svelte';
  import Dot from './Dot.svelte';
  import Pill from './Pill.svelte';
  import ModeToggle from './ModeToggle.svelte';
  import ScopeChip from './ScopeChip.svelte';
  import { status } from '../lib/stores.svelte';

  let label = $derived.by(() => {
    if (!status.online) return 'OFFLINE';
    if (!status.data) return 'CONNECTING';
    return status.data.embedding_available ? 'READY' : 'NO MODEL';
  });
  let tone: 'amber' | 'teal' | 'danger' = $derived.by(() => {
    if (!status.online) return 'danger';
    if (!status.data?.embedding_available) return 'amber';
    return 'teal';
  });
</script>

<header class="topbar">
  <div class="left">
    <span class="mark"><Mark size={22} /></span>
    SATCHEL <span class="sep">::</span>
    <span class="sub">HOST-FREE EMBEDDED LOOKUP</span>
  </div>
  <div class="right">
    <ScopeChip />
    <Pill tone={tone === 'danger' ? 'danger' : tone === 'amber' ? 'amber' : 'teal'}>
      <Dot tone={tone} pulse={!status.data} />
      <span>{label}</span>
    </Pill>
    {#if status.data?.embedding_model}
      <Pill tone="neutral">
        <Dot tone="teal" />
        <span class="model">{status.data.embedding_model}</span>
      </Pill>
    {/if}
    <ModeToggle />
  </div>
</header>

<style>
  .topbar {
    grid-area: topbar;
    position: sticky;
    top: 0;
    z-index: 50;
    background: var(--bg);
    border-bottom: 1px solid var(--border);
    padding: 14px 24px;
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 11px;
    letter-spacing: 1.5px;
    backdrop-filter: blur(8px);
    -webkit-backdrop-filter: blur(8px);
  }
  .left {
    display: flex;
    align-items: center;
    gap: 14px;
    color: var(--amber);
    font-weight: 700;
  }
  .mark {
    display: inline-flex;
    align-items: center;
  }
  .sep {
    color: var(--text-dim);
  }
  .sub {
    color: var(--text-dim);
    font-weight: 400;
    letter-spacing: 2px;
  }
  .right {
    display: flex;
    gap: 12px;
    align-items: center;
    flex-wrap: wrap;
  }
  .model {
    font-size: 10px;
    letter-spacing: 1px;
  }
  @media (max-width: 760px) {
    .sub { display: none; }
    .model { display: none; }
  }
</style>
