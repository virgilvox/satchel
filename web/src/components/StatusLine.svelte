<script lang="ts">
  interface Props {
    text: string;
    tone?: 'dim' | 'teal' | 'danger';
  }
  let { text, tone = 'dim' }: Props = $props();
</script>

<div class="status-line {tone}" aria-live="polite">{text}</div>

<style>
  .status-line {
    font-size: 10px;
    letter-spacing: 1.5px;
    color: var(--text-dim);
    min-height: 1em;
  }
  .status-line.teal { color: var(--teal); }
  .status-line.danger { color: var(--danger); }
</style>
