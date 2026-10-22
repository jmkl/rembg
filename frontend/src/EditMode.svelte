<script lang="ts">
  import { flip } from "svelte/animate";
  import { graph } from "./core/graph.svelte";
  import { defs } from "./core/nodes";
  import { paint } from "./core/paint";
  import Copy from "./components/icon/Copy.svelte";

  let sw = $state(0);
  let sh = $state(0);
  let art: HTMLDivElement;
  let dragEdge = $state("");
  let dragId = $state("");
  let overId = $state("");
  let dropAfter = $state(false);
  let zoom = $state(1);

  type TransformSnapshot = {
    id: string;
    x: number;
    y: number;
    scale: number;
  };

  type DragState = {
    mode: "move" | "scale";
    sx: number;
    sy: number;
    cx: number;
    cy: number;
    d0: number;
    snapshots: TransformSnapshot[];
  };

  let drag: DragState | null = null;

  const W = $derived(Math.max(1, graph.result.cv.width));
  const H = $derived(Math.max(1, graph.result.cv.height));

  const fitK = $derived(
    sw > 0 && sh > 0
      ? Math.max(0.01, Math.min((sw - 48) / W, (sh - 48) / H))
      : 1,
  );

  // Wheel zoom multiplies the fit scale; the stage stays centered.
  const k = $derived(fitK * zoom);

  function onwheel(e: WheelEvent) {
    e.preventDefault();
    const factor = Math.exp(-e.deltaY * 0.0015);
    zoom = Math.min(8, Math.max(0.1, zoom * factor));
  }

  function resetZoom() {
    zoom = 1;
  }

  const sel = $derived(graph.selected);

  const selNode = $derived(
    sel ? graph.nodes.find((n) => n.id === sel) : undefined,
  );

  const selDef = $derived(selNode ? defs[selNode.type as string] : undefined);

  const sp = $derived(sel ? graph.params[sel] : undefined);

  // Return the selected transformable node, or all transformable
  // descendants if the selected node is a mix/group.
  function getTransformTargets(parentId: string): string[] {
    const node = graph.nodes.find((n) => n.id === parentId);
    if (!node) return [];

    if (defs[node.type as string]?.transform) {
      return [parentId];
    }

    const result: string[] = [];
    const visited = new Set<string>();

    function walk(id: string) {
      if (visited.has(id)) return;
      visited.add(id);

      const children = graph.edges
        .filter((e) => e.target === id)
        .map((e) => e.source);

      for (const childId of children) {
        const child = graph.nodes.find((n) => n.id === childId);
        if (!child || !graph.params[childId]) continue;

        if (defs[child.type as string]?.transform) {
          result.push(childId);
        } else {
          walk(childId);
        }
      }
    }

    walk(parentId);
    return result;
  }

  const targets = $derived(sel ? getTransformTargets(sel) : []);

  /** Asset or upstream-input size for a transformable node. */
  function sourceSize(id: string, p: any): { w: number; h: number } | null {
    const input = graph.inputCv(id);
    if (input) return { w: input.width, h: input.height };
    const asset = p.assetId ? graph.assets[p.assetId] : undefined;
    return asset ? { w: asset.w, h: asset.h } : null;
  }

  // Calculate the combined bounding box of the selected layer/group.
  const box = $derived.by(() => {
    void graph.result;
    if (!targets.length) return null;

    const bounds: {
      left: number;
      right: number;
      top: number;
      bottom: number;
    }[] = [];

    for (const id of targets) {
      const p = graph.params[id];
      if (!p || p.visible === false) continue;

      const size = sourceSize(id, p);
      if (!size) continue;

      const width = size.w * Math.max(0.02, p.scale ?? 1);
      const height = size.h * Math.max(0.02, p.scale ?? 1);

      bounds.push({
        left: p.x - width / 2,
        right: p.x + width / 2,
        top: p.y - height / 2,
        bottom: p.y + height / 2,
      });
    }

    if (!bounds.length) return null;

    const left = Math.min(...bounds.map((b) => b.left));
    const right = Math.max(...bounds.map((b) => b.right));
    const top = Math.min(...bounds.map((b) => b.top));
    const bottom = Math.max(...bounds.map((b) => b.bottom));

    return {
      left: left * k,
      top: top * k,
      w: (right - left) * k,
      h: (bottom - top) * k,
      cx: ((left + right) / 2) * k,
      cy: ((top + bottom) / 2) * k,
      rotation:
        targets.length === 1 ? (graph.params[targets[0]]?.rotation ?? 0) : 0,
    };
  });

  function down(e: PointerEvent, mode: "move" | "scale") {
    e.preventDefault();
    e.stopPropagation();

    if (!sel || !targets.length) return;

    const snapshots: TransformSnapshot[] = targets
      .map((id) => {
        const p = graph.params[id];

        return {
          id,
          x: Number(p.x ?? 0),
          y: Number(p.y ?? 0),
          scale: Number(p.scale ?? 1),
        };
      })
      .filter((s) => graph.params[s.id]?.visible !== false);

    if (!snapshots.length) return;

    // Find the group's actual center using all child source bounds.
    const bounds = snapshots.flatMap((s) => {
      const p = graph.params[s.id];
      const size = p ? sourceSize(s.id, p) : null;
      if (!size) return [];

      const w = size.w * s.scale;
      const h = size.h * s.scale;

      return [
        {
          left: s.x - w / 2,
          right: s.x + w / 2,
          top: s.y - h / 2,
          bottom: s.y + h / 2,
        },
      ];
    });

    if (!bounds.length) return;

    const left = Math.min(...bounds.map((b) => b.left));
    const right = Math.max(...bounds.map((b) => b.right));
    const top = Math.min(...bounds.map((b) => b.top));
    const bottom = Math.max(...bounds.map((b) => b.bottom));

    const centerX = (left + right) / 2;
    const centerY = (top + bottom) / 2;

    const rect = art.getBoundingClientRect();
    const cx = rect.left + centerX * k;
    const cy = rect.top + centerY * k;

    drag = {
      mode,
      sx: e.clientX,
      sy: e.clientY,
      cx,
      cy,
      d0: Math.max(1, Math.hypot(e.clientX - cx, e.clientY - cy)),
      snapshots,
    };
  }

  function move(e: PointerEvent) {
    if (!drag) return;

    if (drag.mode === "move") {
      const dx = (e.clientX - drag.sx) / k;
      const dy = (e.clientY - drag.sy) / k;

      // Apply the same delta to every child.
      for (const s of drag.snapshots) {
        const p = graph.params[s.id];
        if (!p) continue;

        p.x = Math.round(s.x + dx);
        p.y = Math.round(s.y + dy);
      }

      return;
    }

    // Scale the whole group around its combined center.
    const distance = Math.hypot(e.clientX - drag.cx, e.clientY - drag.cy);

    const factor = distance / drag.d0;
    const rect = art.getBoundingClientRect();

    const centerX = (drag.cx - rect.left) / k;
    const centerY = (drag.cy - rect.top) / k;

    for (const s of drag.snapshots) {
      const p = graph.params[s.id];
      if (!p) continue;

      p.x = Math.round(centerX + (s.x - centerX) * factor);

      p.y = Math.round(centerY + (s.y - centerY) * factor);

      p.scale = Math.max(0.02, s.scale * factor);
    }
  }

  function endDrag() {
    drag = null;
  }

  type LayerStart = { edge: string; x: number; y: number };
  let layerStart: LayerStart | null = null;
  let layerMoved = false;
  let ignoreLayerClick = false;

  function startLayer(ev: PointerEvent, edgeId: string) {
    if (ev.button !== 0) return;
    if ((ev.target as HTMLElement).closest("button")) return;
    layerStart = { edge: edgeId, x: ev.clientX, y: ev.clientY };
    layerMoved = false;
    ignoreLayerClick = false;
  }

  function moveLayer(ev: PointerEvent) {
    if (!layerStart) return;
    if (!layerMoved) {
      if (
        Math.hypot(ev.clientX - layerStart.x, ev.clientY - layerStart.y) < 4
      ) {
        return;
      }
      layerMoved = true;
      dragEdge = layerStart.edge;
      dragId = layerStart.edge;
    }

    const row = (
      document.elementFromPoint(ev.clientX, ev.clientY) as HTMLElement | null
    )?.closest<HTMLElement>(".layer");
    if (!row || row.dataset.edge === dragEdge) {
      overId = "";
      return;
    }

    const r = row.getBoundingClientRect();
    overId = row.dataset.edge ?? "";
    dropAfter = ev.clientY > r.top + r.height / 2;
  }

  function endLayer() {
    if (layerMoved && dragEdge && overId && overId !== dragEdge) {
      // The panel lists the top layer first, while graph.edges stores layers
      // bottom-first. Visual "after" therefore means "before" in the edge list.
      graph.reorder(dragEdge, overId, !dropAfter);
      ignoreLayerClick = true;
    }
    layerStart = null;
    layerMoved = false;
    dragEdge = "";
    dragId = "";
    overId = "";
    dropAfter = false;
  }

  function selectLayer(nid: string) {
    if (ignoreLayerClick) {
      ignoreLayerClick = false;
      return;
    }
    graph.selected = nid;
  }

  async function copyCanvas(cv: HTMLCanvasElement | null, label: string) {
    if (!cv) {
      graph.toast("Nothing to copy.");
      return;
    }
    try {
      if (!navigator.clipboard?.write) {
        throw new Error("Clipboard images are not supported in this browser.");
      }
      const blob = await new Promise<Blob>((ok, no) =>
        cv.toBlob(
          (b) => (b ? ok(b) : no(new Error("Could not encode image."))),
          "image/png",
        ),
      );
      await navigator.clipboard.write([
        new ClipboardItem({ "image/png": blob }),
      ]);
      graph.toast(`Copied ${label}.`);
    } catch (e) {
      graph.toast(`Could not copy: ${(e as Error).message}`);
    }
  }

  function copyLayer(id: string, label: string) {
    void copyCanvas(graph.evalNode(id), label);
  }

  function copyResult() {
    void copyCanvas(graph.result.cv, "canvas");
  }

  /** Follow Route nodes so they stay out of the layer list. */
  function sourceThroughRoutes(id: string) {
    let current = id;
    const seen = new Set<string>();
    while (current && !seen.has(current)) {
      seen.add(current);
      const node = graph.nodes.find((n) => n.id === current);
      if (!node) return "";
      if (node.type !== "route") return current;
      const input = graph.edges.find((e) => e.target === current);
      if (!input) return "";
      current = input.source;
    }
    return "";
  }

  function layerEdges(parent: string) {
    return [...graph.edges.filter((e) => e.target === parent)]
      .reverse()
      .map((e) => ({ id: e.id, node: sourceThroughRoutes(e.source) }))
      .filter((e) => {
        const node = graph.nodes.find((n) => n.id === e.node);
        return !!e.node && !!graph.params[e.node] && !!defs[node?.type ?? ""];
      });
  }
</script>

<svelte:window
  onpointermove={(ev) => {
    move(ev);
    moveLayer(ev);
  }}
  onpointerup={() => {
    endDrag();
    endLayer();
  }}
  onpointercancel={() => {
    endDrag();
    endLayer();
  }}
/>

<div class="edit">
  <div
    class="stage"
    bind:clientWidth={sw}
    bind:clientHeight={sh}
    onpointerdown={() => (graph.selected = null)}
    {onwheel}
    ondblclick={resetZoom}
    title="Scroll to zoom · Double-click to fit"
    role="presentation"
  >
    <div
      class="art checker"
      bind:this={art}
      style="width:{W * k}px;height:{H * k}px"
    >
      <canvas use:paint={graph.result.cv}></canvas>

      {#if box}
        <div
          class="box"
          role="presentation"
          onpointerdown={(e) => down(e, "move")}
          style="
            left:{box.left}px;
            top:{box.top}px;
            width:{box.w}px;
            height:{box.h}px;
            transform:rotate({box.rotation}deg)
          "
        >
          <i
            class="handle"
            role="presentation"
            onpointerdown={(e) => down(e, "scale")}
          ></i>
        </div>
      {/if}
    </div>

    <div
      class="zoom-pill"
      role="presentation"
      onpointerdown={(e) => e.stopPropagation()}
    >
      <button
        title="Zoom out"
        onclick={() => (zoom = Math.max(0.1, zoom / 1.2))}>−</button
      >
      <button
        class="zoom-reset"
        title="Reset zoom (double-click canvas)"
        onclick={resetZoom}>{Math.round(zoom * 100)}%</button
      >
      <button title="Zoom in" onclick={() => (zoom = Math.min(8, zoom * 1.2))}
        >+</button
      >
    </div>
  </div>

  <aside class="panel">
    <h3>
      <span>Layers <small>top → bottom</small></span>
      <button
        class="copy-all"
        title="Copy entire canvas image"
        aria-label="Copy entire canvas image"
        onclick={copyResult}
      >
        <Copy />
      </button>
    </h3>

    <div class="layers">
      {#snippet tree(parent: string, depth: number, seen: Set<string>)}
        {#each layerEdges(parent) as e (e.id)}
          {@const nid = e.node}
          {@const p = graph.params[nid]}
          {@const d = defs[graph.nodes.find((n) => n.id === nid)?.type ?? ""]}

          <div class="layer-wrap" animate:flip={{ duration: 160 }}>
            {#if p && d && !seen.has(nid)}
              <div
                class="layer"
                class:sel={graph.selected === nid}
                class:dragging={dragId === e.id}
                class:over={overId === e.id && dragId !== e.id}
                class:drop-before={overId === e.id &&
                  dragId !== e.id &&
                  !dropAfter}
                class:drop-after={overId === e.id &&
                  dragId !== e.id &&
                  dropAfter}
                style:padding-left="{8 + depth * 16}px"
                data-edge={e.id}
                role="presentation"
                title="Drag to reorder"
                onpointerdown={(ev) => startLayer(ev, e.id)}
                onclick={() => selectLayer(nid)}
              >
                <button
                  title="Toggle visibility"
                  onclick={(ev) => {
                    ev.stopPropagation();
                    p.visible = p.visible === false;
                  }}
                >
                  {p.visible === false ? "○" : "●"}
                </button>

                <span class="name" style:--c={d.color}>
                  {p.name ?? d.title}
                </span>

                <button
                  title="Copy layer image"
                  aria-label="Copy layer image"
                  onclick={(ev) => {
                    ev.stopPropagation();
                    copyLayer(nid, p.name ?? d.title);
                  }}
                >
                  <Copy />
                </button>

                <button
                  title="Delete"
                  onclick={(ev) => {
                    ev.stopPropagation();
                    graph.remove(nid);
                  }}
                >
                  ✕
                </button>
              </div>

              {#if d.inputs.length}
                {@render tree(nid, depth + 1, new Set([...seen, nid]))}
              {/if}
            {/if}
          </div>
        {/each}
      {/snippet}

      {@render tree(graph.viewerId, 0, new Set())}
    </div>

    {#if sp && selDef}
      <h3>{sp.name ?? selDef.title}</h3>

      <label class="field">
        <span>Opacity</span>
        <input
          type="range"
          min="0"
          max="1"
          step="0.01"
          bind:value={graph.params[sel!].opacity}
        />
      </label>

      {#if selDef.transform}
        {#each [["x", "X", 1], ["y", "Y", 1], ["scale", "Scale", 0.01], ["rotation", "Rotate°", 1]] as [key, label, step] (key)}
          <label class="field">
            <span>{label}</span>
            <input
              type="number"
              {step}
              bind:value={graph.params[sel!][key as string]}
            />
          </label>
        {/each}
      {:else if targets.length}
        <p class="hint">
          Group transform: {targets.length} layer(s)
        </p>
        <p class="hint">
          Drag the group to move all its children. Drag the corner handle to
          scale them together.
        </p>
      {/if}

      {#each selDef.fields ?? [] as f (f.key)}
        <label class="field">
          <span>{f.label}</span>

          {#if f.type === "select"}
            <select bind:value={graph.params[sel!][f.key]}>
              {#each f.options ?? [] as option}
                <option value={option.value}>
                  {option.label}
                </option>
              {/each}
            </select>
          {:else}
            <input
              type={f.type}
              min={f.min}
              max={f.max}
              step={f.step ?? 1}
              bind:value={graph.params[sel!][f.key]}
            />
          {/if}
        </label>
      {/each}
    {/if}
  </aside>
</div>

<style>
  .edit {
    display: flex;
    width: 100%;
    flex: 1 1 0;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    background: var(--bg);
  }

  .stage {
    position: relative;
    display: flex;
    flex: 1;
    min-width: 0;
    min-height: 0;
    align-items: center;
    justify-content: center;
    overflow: hidden;
    background: radial-gradient(
      circle at 50% 42%,
      color-mix(in srgb, var(--panel) 78%, var(--bg)) 0,
      var(--bg) 74%
    );
    cursor: default;
  }

  .art {
    position: relative;
    flex: none;
    overflow: hidden;
    box-shadow:
      0 0 0 1px var(--line),
      0 18px 60px #0008;
  }

  canvas {
    display: block;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }

  .box {
    position: absolute;
    z-index: 2;
    box-sizing: border-box;
    border: 1px solid var(--acc);
    box-shadow: 0 0 0 1px #0008;
    cursor: move;
    touch-action: none;
  }

  .handle {
    position: absolute;
    right: -6px;
    bottom: -6px;
    width: 12px;
    height: 12px;
    box-sizing: border-box;
    border: 1px solid #fff;
    border-radius: 2px;
    background: var(--acc);
    cursor: nwse-resize;
    touch-action: none;
  }

  .zoom-pill {
    position: absolute;
    left: 12px;
    bottom: 12px;
    z-index: 3;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 3px;
    border: 1px solid var(--line);
    border-radius: 8px;
    background: color-mix(in srgb, var(--panel) 88%, transparent);
    box-shadow: 0 6px 20px #0006;
    backdrop-filter: blur(8px);
  }

  .zoom-pill button {
    min-width: 24px;
    height: 24px;
    padding: 0 6px;
    border: 0;
    border-radius: 5px;
    background: transparent;
    color: var(--fg);
    font-size: 11px;
    cursor: pointer;
  }

  .zoom-pill button:hover:not(:disabled) {
    border: 0;
    background: color-mix(in srgb, var(--fg) 10%, transparent);
  }

  .zoom-pill .zoom-reset {
    min-width: 46px;
    color: var(--mut);
    font-variant-numeric: tabular-nums;
  }

  .panel {
    display: flex;
    flex: 0 0 280px;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
    padding: 12px;
    overflow: auto;
    border-left: 1px solid var(--line);
    background: var(--panel);
    scrollbar-color: var(--line) transparent;
    scrollbar-width: thin;
  }

  .panel::-webkit-scrollbar {
    width: 8px;
  }

  .panel::-webkit-scrollbar-thumb {
    border-radius: 999px;
    background: var(--line);
  }

  h3 {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: 8px;
    margin: 2px 0;
    padding-bottom: 6px;
    border-bottom: 1px solid var(--line);
    color: var(--fg);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  h3 small {
    color: var(--mut);
    font-size: 10px;
    font-weight: 400;
    letter-spacing: 0;
    text-transform: none;
  }

  h3 .copy-all {
    flex: none;
    padding: 3px 7px;
    border: 1px solid var(--line);
    border-radius: 5px;
    background: color-mix(in srgb, var(--fg) 5%, transparent);
    color: var(--mut);
    font-size: 10px;
    font-weight: 500;
    letter-spacing: 0;
    text-transform: none;
    cursor: pointer;
  }

  h3 .copy-all:hover {
    border-color: var(--acc);
    color: var(--fg);
  }

  .layers {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .layer-wrap {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .layer {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 30px;
    padding: 2px 4px;
    border: 1px solid transparent;
    border-radius: 6px;
    cursor: grab;
    user-select: none;
    transition:
      background 0.12s ease,
      border-color 0.12s ease,
      box-shadow 0.12s ease,
      opacity 0.12s ease,
      transform 0.12s ease;
  }

  .layer:hover {
    background: color-mix(in srgb, var(--fg) 6%, transparent);
  }

  .layer.sel {
    border-color: color-mix(in srgb, var(--acc) 55%, transparent);
    background: color-mix(in srgb, var(--acc) 20%, var(--panel));
  }

  .layer.dragging {
    border-style: dashed;
    border-color: var(--acc);
    opacity: 0.38;
    transform: scale(0.98);
    cursor: grabbing;
  }

  .layer.over {
    background: color-mix(in srgb, var(--acc) 12%, transparent);
  }

  .layer.drop-before {
    box-shadow: inset 0 2px 0 var(--acc);
  }

  .layer.drop-after {
    box-shadow: inset 0 -2px 0 var(--acc);
  }

  .layer button {
    display: grid;
    flex: none;
    place-items: center;
    width: 22px;
    height: 22px;
    padding: 0;
    border: 0;
    border-radius: 4px;
    background: transparent;
    color: var(--mut);
    font-size: 11px;
    cursor: pointer;
    transition:
      background 0.12s ease,
      color 0.12s ease;
  }

  .layer button:hover:not(:disabled) {
    border: 0;
    background: color-mix(in srgb, var(--fg) 12%, transparent);
    color: var(--fg);
  }

  .layer.sel button {
    color: color-mix(in srgb, var(--fg) 75%, var(--acc));
  }

  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    padding-left: 6px;
    border-left: 3px solid var(--c, var(--line));
    color: var(--fg);
    font-size: 12px;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .field {
    display: grid;
    gap: 4px;
    align-items: stretch;
    justify-content: initial;
    font-size: 11px;
  }

  .field span {
    min-width: 0;
    color: var(--mut);
    opacity: 1;
  }

  .field input,
  .field select {
    flex: none;
    width: 100%;
    min-width: 0;
    box-sizing: border-box;
  }

  .hint {
    margin: 0;
    color: var(--mut);
    font-size: 11px;
    line-height: 1.5;
  }

  /* In a narrow split pane, put the inspector under the preview so the
     artwork keeps a usable size. */
  @container (max-width: 560px) {
    .edit {
      flex-direction: column;
    }

    .panel {
      flex: 0 0 42%;
      border-top: 1px solid var(--line);
      border-left: 0;
    }
  }
</style>
