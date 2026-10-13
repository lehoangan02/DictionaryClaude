<script lang="ts">
  import Drafts from './Drafts.svelte';
  let reviewing = $state(false);
</script>

{#if !reviewing}
  <h1>Claude</h1>
  <p class="muted">Photograph your tutor notes. Your server reads them first; then check the reading with Claude here, line by line, and save the result as a note.</p>
{/if}
<Drafts bind:reviewing />
