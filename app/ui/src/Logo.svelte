<script lang="ts">
  // The LadyBird mark (rounded app icon).
  let { size = 36 }: { size?: number } = $props();
</script>

<img src="./ladybird.png" width={size} height={size} alt="LadyBird" style="border-radius:{size * 0.22}px; box-shadow:0 2px 6px rgba(16,40,80,.25)" />
