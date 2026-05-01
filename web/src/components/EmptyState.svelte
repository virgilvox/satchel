<script lang="ts">
  interface Props {
    text?: string;
    glyph?: string;
  }
  let { text = 'nothing here', glyph = '::' }: Props = $props();
</script>

<div class="empty"><span class="glyph">{glyph}</span>{text}</div>
