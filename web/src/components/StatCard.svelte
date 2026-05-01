<script lang="ts">
  interface Props {
    label: string;
    value: string | number;
    meta?: string;
    accent?: boolean;
  }
  let { label, value, meta, accent = true }: Props = $props();
</script>

<div class="stat-card" data-label={label}>
  <span class="value" class:dim={!accent}>{value}</span>
  {#if meta}<span class="meta">{meta}</span>{/if}
</div>

<style>
  .stat-card {
    background: var(--surface);
    border: 1px solid var(--border);
    padding: 22px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    position: relative;
  }
  .stat-card::before {
    content: attr(data-label);
    position: absolute;
    top: -7px;
    left: 14px;
    background: var(--bg);
    padding: 0 8px;
    font-size: 9px;
    letter-spacing: 2px;
    color: var(--text-dim);
  }
  .value {
    font-size: 32px;
    font-weight: 700;
    color: var(--amber);
    letter-spacing: 2px;
    line-height: 1;
  }
  .value.dim { color: var(--text-bright); }
  .meta {
    font-size: 10px;
    color: var(--text-dim);
    letter-spacing: 1px;
  }
</style>
