<script lang="ts">
  import { graph } from "./core/graph.svelte";
  import { paint } from "./core/paint";

  let { id }: { id: string } = $props();
  const p = $derived(graph.params[id]);

  // These maps live outside $state, so dataRev + result drive reactivity.
  // graph.result also covers upstream input changes (the source may be a canvas).
  const source = $derived((graph.result, graph.imageSource(id)));
  const input = $derived((graph.result, graph.inputCv(id)));
  const hasMask = $derived((graph.dataRev, graph.result, graph.hasMask(id)));
  const ready = $derived((graph.dataRev, graph.result, graph.maskReady(id)));
  const masked = $derived(
    (graph.dataRev,
    graph.result,
    p?.useMask && ready ? graph.maskedSource(id) : null),
  );
  const size = $derived(
    masked
      ? `${masked.width} × ${masked.height}`
      : source
        ? `${source.w} × ${source.h}`
        : "",
  );
  const asset = $derived(p?.assetId ? graph.assets[p.assetId] : undefined);
</script>

{#if source}
  <div class="thumb checker">
    {#if masked}
      <canvas use:paint={masked}></canvas>
    {:else if input}
      <canvas use:paint={input}></canvas>
    {:else if asset}
      <img src={asset.url} alt={asset.name} draggable="false" />
    {/if}
  </div>

  <div class="meta">
    <span>
      {size} · {Math.round((p?.scale ?? 1) * 100)}%{input ? " · input" : ""}
    </span>
    {#if hasMask}
      <span class="pill" class:on={p?.useMask && ready}>
        {ready ? (p?.useMask ? "mask on" : "mask off") : "mask stale"}
      </span>
    {/if}
  </div>

  <button
    class="act nodrag"
    disabled={p?.busy}
    onclick={() => graph.removeBg(id)}
  >
    {#if p?.busy}
      Removing background…
    {:else if hasMask}
      ↻ Re-run background removal
    {:else}
      ✦ Remove background
    {/if}
  </button>

  {#if hasMask}
    <label class="field nodrag">
      <span>Use AI mask</span>
      <input
        type="checkbox"
        checked={p?.useMask}
        disabled={p?.busy || !ready}
        onchange={(e) => (graph.params[id].useMask = e.currentTarget.checked)}
      />
    </label>
  {/if}
{/if}
