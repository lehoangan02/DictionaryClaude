<script lang="ts">
  import Icon from './Icon.svelte';
  import { onMount } from 'svelte';

  // A camera for computers: browsers only open the camera from a file input on phones.
  let { onshot, onclose }: { onshot: (f: File) => void; onclose: () => void } = $props();
  let video: HTMLVideoElement;
  let stream: MediaStream | null = null;
  let error = $state('');
  let ready = $state(false);
  // A notebook held sideways comes out turned; rotate the preview and the photo together.
  let turn = $state(0);

  onMount(() => {
    navigator.mediaDevices
      .getUserMedia({ video: { facingMode: 'environment', width: { ideal: 2560 }, height: { ideal: 1440 } }, audio: false })
      .then((s) => { stream = s; video.srcObject = s; return video.play(); })
      .then(() => { ready = true; })
      .catch((e) => { error = `The camera could not be opened (${e?.name ?? 'error'}). Allow LadyBird in System Settings → Privacy & Security → Camera, or choose a photo instead.`; });
    return () => stream?.getTracks().forEach((t) => t.stop());
  });

  function picked(e: Event) {
    const f = (e.target as HTMLInputElement).files?.[0];
    if (f) onshot(f);
    onclose();
  }

  function shoot() {
    const c = document.createElement('canvas');
    const sideways = turn % 180 !== 0;
    c.width = sideways ? video.videoHeight : video.videoWidth;
    c.height = sideways ? video.videoWidth : video.videoHeight;
    const g = c.getContext('2d')!;
    g.translate(c.width / 2, c.height / 2);
    g.rotate((turn * Math.PI) / 180);
    g.drawImage(video, -video.videoWidth / 2, -video.videoHeight / 2);
    c.toBlob((b) => { if (b) onshot(new File([b], 'photo.jpg', { type: 'image/jpeg' })); onclose(); }, 'image/jpeg', 0.92);
  }
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onclose()} />
<div class="cam" role="dialog" aria-label="Camera">
  <!-- svelte-ignore a11y_media_has_caption -->
  <div class="stage"><video bind:this={video} playsinline muted style="transform: rotate({turn}deg){turn % 180 ? ' scale(0.62)' : ''}"></video></div>
  <button class="x" onclick={onclose} aria-label="Close camera"><Icon name="close" size={22} /></button>
  {#if error}<p class="err">{error}</p>{/if}
  <div class="bar">
    <button class="btn" onclick={onclose}>Cancel</button>
    {#if !error}<button class="btn" onclick={() => (turn = (turn + 90) % 360)}>⟳ Rotate</button>{/if}
    {#if error}
      <label class="btn primary">Choose a photo<input type="file" accept="image/*" onchange={picked} hidden /></label>
    {:else}
      <button class="btn primary" disabled={!ready} onclick={shoot}>Take photo</button>
    {/if}
  </div>
</div>

<style>
  .cam { position: fixed; inset: 0; z-index: 100; background: #000; display: grid; grid-template-rows: minmax(0, 1fr) auto; height: 100dvh; overflow: hidden; }
  .stage { min-height: 0; display: grid; place-items: center; overflow: hidden; }
  video { max-width: 100%; max-height: 100%; width: auto; height: auto; object-fit: contain; transition: transform var(--t) var(--ease); }
  .x { position: absolute; top: calc(12px + env(safe-area-inset-top)); right: 14px; width: 44px; height: 44px; border-radius: 50%; border: 0; background: rgba(255, 255, 255, 0.9); color: #111; font-size: 20px; z-index: 2; }
  .bar { display: flex; gap: 12px; justify-content: center; padding: 16px; padding-bottom: calc(16px + env(safe-area-inset-bottom)); }
  .err { position: absolute; left: 16px; right: 16px; top: 24px; }
</style>
