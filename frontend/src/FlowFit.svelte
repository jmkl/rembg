<script lang="ts">
  import { onMount } from "svelte";
  import { useSvelteFlow } from "@xyflow/svelte";
  import { graph } from "./core/graph.svelte";

  const { fitView } = useSvelteFlow();
  let last = graph.mode;
  let lastFit = graph.fitRev;

  // The node canvas changes size when entering or leaving Split. Fit once
  // afterwards, but never while the divider is being dragged (that would
  // fight the user).
  function fit() {
    if (!graph.nodes.length) return;
    void fitView({ padding: 0.12, duration: 180 });
  }

  function scheduleFit() {
    requestAnimationFrame(() => requestAnimationFrame(fit));
  }

  onMount(scheduleFit);

  $effect(() => {
    const mode = graph.mode;
    const fitRev = graph.fitRev;
    if (mode !== last) {
      last = mode;
      if (mode !== "edit") scheduleFit();
    }
    if (fitRev !== lastFit) {
      lastFit = fitRev;
      scheduleFit();
    }
  });
</script>
