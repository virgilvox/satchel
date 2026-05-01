<script lang="ts">
  interface Props {
    num: string;
    title: string;
    desc?: string;
  }
  let { num, title, desc }: Props = $props();
</script>

<div class="view-head">
  <div class="num">{num}</div>
  <div class="title">{@html title}</div>
  {#if desc}<div class="desc">{desc}</div>{/if}
</div>

<style>
  .view-head {
    display: flex;
    align-items: baseline;
    gap: 18px;
    padding-bottom: 18px;
    margin-bottom: 28px;
    border-bottom: 1px dashed var(--border);
  }
  .num {
    font-size: 36px;
    font-weight: 700;
    line-height: 1;
    color: var(--amber);
    letter-spacing: 2px;
  }
  .title {
    font-size: 13px;
    letter-spacing: 4px;
    text-transform: uppercase;
    color: var(--text-bright);
    font-weight: 700;
  }
  .title :global(.slash) { color: var(--text-dim); margin: 0 10px; }
  .desc {
    font-size: 12px;
    color: var(--text-dim);
    margin-left: auto;
    max-width: 380px;
    line-height: 1.6;
  }
  @media (max-width: 760px) {
    .desc { display: none; }
  }
</style>
