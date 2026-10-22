<script lang="ts">
  // ONE component renders every node type from its definition in core/nodes.ts.
  import { Handle, Position, type NodeProps } from "@xyflow/svelte";
  import { graph } from "./core/graph.svelte";

  import { defs } from "./core/nodes";
  import { paint } from "./core/paint";
  import ImageBody from "./ImageBody.svelte";
  import FaceRestoreBody from "./FaceRestoreBody.svelte";
  import CurvesBody from "./CurvesBody.svelte";

  let { id }: NodeProps = $props();

  // Custom bodies are optional; plain nodes just use `fields`.
  const bodies: Record<string, any> = {
    image: ImageBody,
    facerestore: FaceRestoreBody,
    curves: CurvesBody,
  };

  const def = $derived(
    defs[graph.nodes.find((n) => n.id === id)?.type ?? "mix"],
  );
  const p = $derived(graph.params[id]);
  const Body = $derived(bodies[def.type]);
  // Evaluate this node itself so its preview also updates while it is not
  // connected to the Viewer. evaluate() memoizes by params/inputs.
  const out = $derived(graph.evalNode(id));
</script>

{#if p}
  <div
    class="node"
    class:route={def.type === "route"}
    class:sel={graph.selected === id}
    style:--c={def.color}
    title={def.title}
  >
    {#if def.type !== "route"}
      <header>{p.name ?? def.title}</header>
    {/if}

    {#each def.inputs as s, i (s.id)}
      <Handle
        type="target"
        position={Position.Left}
        id={s.id}
        style="top:{def.type === 'route' ? '50%' : `${44 + i * 22}px`}"
      />
    {/each}
    {#if def.output}<Handle
        type="source"
        position={Position.Right}
        id="out"
      />{/if}

    {#if def.type !== "route"}
      <div class="body">
        {#if Body}<Body {id} />{/if}

      {#each def.fields ?? [] as f (f.key)}
        <label class="field nodrag">
          <span>{f.label}</span>
          {#if f.type === "color"}
            <input type="color" bind:value={graph.params[id][f.key]} />
          {:else if f.type === "range"}
            <input
              type="range"
              min={f.min}
              max={f.max}
              step={f.step ?? 1}
              bind:value={graph.params[id][f.key]}
            />
          {:else if f.type === "select"}
            <select
              value={graph.params[id][f.key]}
              onchange={(e) =>
                (graph.params[id][f.key] = e.currentTarget.value)}
            >
              {#each f.options ?? [] as option}
                <option value={option.value}>{option.label}</option>
              {/each}
            </select>
          {:else}
            <input
              type="number"
              min={f.min}
              max={f.max}
              step={f.step ?? 1}
              bind:value={graph.params[id][f.key]}
            />
          {/if}
        </label>
      {/each}

        {#if def.type !== "image"}<canvas class="thumb checker" use:paint={out}
          ></canvas>{/if}
      </div>
    {/if}
  </div>
{/if}
