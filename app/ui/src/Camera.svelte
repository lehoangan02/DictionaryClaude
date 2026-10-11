<script lang="ts">
  import { onMount } from 'svelte';

  // A camera for computers: browsers only open the camera from a file input on phones.
  let { onshot, onclose }: { onshot: (f: File) => void; onclose: () => void } = $props();
  let video: HTMLVideoElement;
  let stream: MediaStream | null = null;
  let error = $state('');
  let ready = $state(false);

  onMount(() => {
    navigator.mediaDevices
      .getUserMedia({ video: { facingMode: 'environment', width: { ideal: 2560 }, height: { ideal: 1440 } }, audio: false })
      .then((s) => { stream = s; video.srcObject = s; return video.play(); })
      .then(() => { ready = true; })
      .catch(() => { error = 'No camera is available, or access was refused. Allow it in System Settings → Privacy & Security → Camera, or use “Add photos”.'; });
    return () => stream?.getTracks().forEach((t) => t.stop());
  });

  function shoot() {
    const c = document.createElement('canvas');
    c.width = video.videoWidth; c.height = video.videoHeight;
    c.getContext('2d')!.drawImage(video, 0, 0);
    c.toBlob((b) => { if (b) onshot(new File([b], 'photo.jpg', { type: 'image/jpeg' })); onclose(); }, 'image/jpeg', 0.92);
  }
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && onclose()} />
<div class="cam" role="dialog" aria-label="Camera">
  <!-- svelte-ignore a11y_media_has_caption -->
  <video bind:this={video} playsinline muted></video>
  {#if error}<p class="err">{error}</p>{/if}
  <div class="bar">
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={!ready} onclick={shoot}>Take photo</button>
  </div>
</div>

<style>
  .cam { position: fixed; inset: 0; z-index: 100; background: #000; display: grid; grid-template-rows: 1fr auto; }
  video { width: 100%; height: 100%; object-fit: contain; min-height: 0; }
  .bar { display: flex; gap: 12px; justify-content: center; padding: 16px; padding-bottom: calc(16px + env(safe-area-inset-bottom)); }
  .err { position: absolute; left: 16px; right: 16px; top: 24px; }
</style>
