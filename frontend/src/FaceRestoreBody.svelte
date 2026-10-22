<script lang="ts">
  import { graph } from "./core/graph.svelte";

  let { id }: { id: string } = $props();
  const p = $derived(graph.params[id]);
  const state = $derived(
    (graph.result, graph.dataRev, graph.restoreState(id)),
  );
</script>

<button
  class="act nodrag"
  class:primary={state === "none" || state === "stale"}
  disabled={p?.busy || state === "empty"}
  onclick={() => graph.restoreFaces(id)}
>
  {#if p?.busy}
    Restoring faces…
  {:else if state === "ready"}
    ↻ Re-run face restore
  {:else if state === "stale"}
    ↻ Restore again
  {:else}
    ✦ Restore faces
  {/if}
</button>

{#if state === "ready"}
  <div class="seg face-view nodrag" role="group" aria-label="Face restore view">
    <button
      class:on={!p?.showOriginal}
      aria-pressed={!p?.showOriginal}
      onclick={() => graph.setFaceView(id, false)}>Restored</button
    >
    <button
      class:on={!!p?.showOriginal}
      aria-pressed={!!p?.showOriginal}
      onclick={() => graph.setFaceView(id, true)}>Original</button
    >
  </div>
{/if}

{#if state === "empty"}
  <div class="meta"><span>Connect an image first</span></div>
{:else if state === "stale"}
  <div class="meta"><span>Input or settings changed</span></div>
{/if}
