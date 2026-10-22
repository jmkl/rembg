<script lang="ts">
  import { graph } from "./core/graph.svelte";
  import {
    curveSampler,
    identityCurve,
    sanitizeCurve,
    type CurvePoint,
  } from "./core/nodes";

  type Channel = "rgb" | "red" | "green" | "blue";
  type CurveStore = Record<Channel, CurvePoint[]>;

  let { id }: { id: string } = $props();

  const p = $derived(graph.params[id]);
  const channel = $derived(
    (["rgb", "red", "green", "blue"].includes(p?.channel)
      ? p.channel
      : "rgb") as Channel,
  );
  const points = $derived(
    sanitizeCurve(p?.curves?.[channel] ?? identityCurve()),
  );

  let svg = $state<SVGSVGElement | null>(null);
  let dragIndex = $state<number | null>(null);

  const W = 256;
  const H = 152;
  const labels: Record<Channel, string> = {
    rgb: "RGB",
    red: "Red",
    green: "Green",
    blue: "Blue",
  };
  const colors: Record<Channel, string> = {
    rgb: "#e5e7eb",
    red: "#ef4444",
    green: "#22c55e",
    blue: "#3b82f6",
  };

  const path = $derived.by(() => {
    const sample = curveSampler(points);
    let d = "";
    for (let i = 0; i <= 96; i++) {
      const x = i / 96;
      d += `${i === 0 ? "M" : "L"}${(x * W).toFixed(2)} ${(
        (1 - sample(x)) *
        H
      ).toFixed(2)} `;
    }
    return d.trim();
  });

  function ensureStore(): CurveStore {
    const params = graph.params[id];
    if (!params.curves || typeof params.curves !== "object") {
      params.curves = {};
    }
    for (const key of ["rgb", "red", "green", "blue"] as Channel[]) {
      if (!Array.isArray(params.curves[key])) {
        params.curves[key] = identityCurve();
      }
    }
    return params.curves as CurveStore;
  }

  function commit(next: CurvePoint[]) {
    const params = graph.params[id];
    const store = ensureStore();
    store[channel] = next;
    // Reassign the params object so the graph evaluator sees a new signature
    // even when a nested curve point was dragged.
    graph.params[id] = {
      ...params,
      curves: { ...store, [channel]: next },
    };
  }

  function setChannel(value: string) {
    const params = graph.params[id];
    ensureStore();
    graph.params[id] = { ...params, channel: value as Channel };
  }

  function positionAt(ev: MouseEvent | PointerEvent) {
    if (!svg) return { x: 0, y: 0 };
    const rect = svg.getBoundingClientRect();
    return {
      x: Math.max(0, Math.min(1, (ev.clientX - rect.left) / rect.width)),
      y: Math.max(0, Math.min(1, 1 - (ev.clientY - rect.top) / rect.height)),
    };
  }

  function pointAt(ev: MouseEvent | PointerEvent) {
    if (!svg) return -1;
    const rect = svg.getBoundingClientRect();
    const pos = positionAt(ev);
    let best = -1;
    let bestDist = 10;
    for (let i = 0; i < points.length; i++) {
      const dist = Math.hypot(
        (points[i].x - pos.x) * rect.width,
        (points[i].y - pos.y) * rect.height,
      );
      if (dist < bestDist) {
        bestDist = dist;
        best = i;
      }
    }
    return best;
  }

  function startDrag(index: number, ev: PointerEvent) {
    dragIndex = index;
    svg?.setPointerCapture(ev.pointerId);
  }

  function onDown(ev: PointerEvent) {
    if (ev.button !== 0) return;
    ev.preventDefault();
    ev.stopPropagation();
    const hit = pointAt(ev);
    if (hit >= 0) {
      startDrag(hit, ev);
      return;
    }
    const pos = positionAt(ev);
    // The endpoints stay on the left/right edges; new points live inside.
    const point = {
      x: Math.max(0.01, Math.min(0.99, pos.x)),
      y: pos.y,
    };
    const list = [...points, point].sort((a, b) => a.x - b.x);
    const index = list.indexOf(point);
    commit(list);
    startDrag(index, ev);
  }

  function onMove(ev: PointerEvent) {
    if (dragIndex === null) return;
    ev.preventDefault();
    ev.stopPropagation();
    const pos = positionAt(ev);
    const list = points.map((point) => ({ ...point }));
    const i = dragIndex;
    if (i < 0 || i >= list.length) return;
    let x = pos.x;
    if (i === 0) x = 0;
    else if (i === list.length - 1) x = 1;
    else {
      x = Math.max(list[i - 1].x + 0.005, Math.min(list[i + 1].x - 0.005, x));
    }
    list[i] = { x, y: pos.y };
    commit(list);
  }

  function onUp(ev: PointerEvent) {
    if (dragIndex === null) return;
    dragIndex = null;
    try {
      svg?.releasePointerCapture(ev.pointerId);
    } catch {
      // The browser may have released the pointer already.
    }
  }

  function onContextMenu(ev: MouseEvent) {
    ev.preventDefault();
    ev.stopPropagation();
    const hit = pointAt(ev);
    if (hit <= 0 || hit >= points.length - 1) return;
    commit(points.filter((_, i) => i !== hit));
  }

  function reset() {
    commit(identityCurve());
  }
</script>

<div class="curve-editor nodrag">
  <div class="curve-head">
    <label class="field">
      <span>Channel</span>
      <select
        value={channel}
        onchange={(e) => setChannel(e.currentTarget.value)}
      >
        {#each Object.entries(labels) as [value, label]}
          <option value={value}>{label}</option>
        {/each}
      </select>
    </label>
    <button
      class="curve-reset"
      type="button"
      onclick={reset}
      title="Reset {labels[channel]} curve"
      aria-label="Reset {labels[channel]} curve"
    >
      ↺
    </button>
  </div>

  <svg
    bind:this={svg}
    class="curve-canvas"
    viewBox={`0 0 ${W} ${H}`}
    role="img"
    aria-label="{labels[channel]} curve editor"
    onpointerdown={onDown}
    onpointermove={onMove}
    onpointerup={onUp}
    onpointercancel={onUp}
    oncontextmenu={onContextMenu}
  >
    <rect x="0" y="0" width={W} height={H} fill="transparent" />
    {#each [0.25, 0.5, 0.75] as grid}
      <line x1={grid * W} y1="0" x2={grid * W} y2={H} class="curve-grid" />
      <line x1="0" y1={grid * H} x2={W} y2={grid * H} class="curve-grid" />
    {/each}
    <line x1="0" y1={H} x2={W} y2="0" class="curve-diag" />
    <path d={path} class="curve-path" stroke={colors[channel]} />
    {#each points as point, i (i)}
      <circle
        cx={point.x * W}
        cy={(1 - point.y) * H}
        r="4.4"
        class="curve-point"
        stroke={colors[channel]}
      />
    {/each}
  </svg>
  <div class="curve-hint">Click to add · drag points · right-click to remove</div>
</div>
