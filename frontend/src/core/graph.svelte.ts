import type { Edge, Node } from "@xyflow/svelte";
import { clearEvalCache, evaluate } from "./evaluate";
import { fetchUrl, mask, restore } from "./api";
import {
  blank,
  defs,
  type Asset,
  type Ctx,
  type Cv,
  type ImageSource,
} from "./nodes";

export type SnapshotNode = {
  id: string;
  type: string;
  position: { x: number; y: number };
  data: Record<string, any>;
  zIndex?: number;
  draggable?: boolean;
  deletable?: boolean;
  width?: number;
  height?: number;
};
export type SnapshotEdge = {
  id: string;
  source: string;
  target: string;
  sourceHandle?: string | null;
  targetHandle?: string | null;
};
export type GraphData = {
  nodes: SnapshotNode[];
  edges: SnapshotEdge[];
  params: Record<string, any>;
};
export type GraphSnapshot = GraphData & {
  assets: Asset[];
  masks: { nodeId: string; cv: Cv }[];
  restores: {
    nodeId: string;
    cv: Cv;
    strength: number;
    onlyCenter: boolean;
  }[];
};
export type WorkspaceData = {
  assets: Asset[];
  masks: { nodeId: string; cv: Cv }[];
  restores: {
    nodeId: string;
    cv: Cv;
    strength: number;
    onlyCenter: boolean;
  }[];
};

let seq = 0;
const uid = (p: string) =>
  `${p}${++seq}_${Math.random().toString(36).slice(2, 6)}`;
const load = (url: string) =>
  new Promise<HTMLImageElement>((ok, no) => {
    const i = new Image();
    i.onload = () => ok(i);
    i.onerror = () => no(new Error("Could not decode that image."));
    i.src = url;
  });

// AI masks + face restores live outside reactive state (big canvases);
// dataRev feeds the evaluator/UI cache keys.
const masks = new Map<string, Cv>();
const maskKey = new Map<string, unknown>();
const maskVer = new Map<string, number>();
const masked = new Map<string, { v: number; key: unknown; cv: Cv }>();
const restores = new Map<
  string,
  { src: Cv; cv: Cv; strength: number; onlyCenter: boolean }
>();
const restoreVer = new Map<string, number>();
let version = 0;

class Graph {
  nodes = $state.raw<Node[]>([]);
  edges = $state.raw<Edge[]>([]);
  params = $state<Record<string, any>>({});
  assets = $state.raw<Record<string, Asset>>({});
  // "split" = vertical divider (left/right); "splith" = horizontal divider
  // (top/bottom). Start stacked so the graph and editor are both visible.
  mode = $state<"node" | "split" | "splith" | "edit">("splith");
  selected = $state<string | null>(null);
  dataRev = $state(0);
  fitRev = $state(0);
  message = $state("");

  viewerId = $derived(this.nodes.find((n) => n.type === "viewer")?.id ?? "");

  /** Evaluation context for the Viewer and for saved-data settlement. */
  private context(): Ctx {
    const vp = this.params[this.viewerId];
    const w = Math.min(4096, Math.max(16, Math.round(vp?.w) || 1280));
    const h = Math.min(4096, Math.max(16, Math.round(vp?.h) || 720));
    return {
      w,
      h,
      assets: this.assets,
      rev: (id) => (maskVer.get(id) ?? 0) + (restoreVer.get(id) ?? 0),
      mask: (id, source) => this.maskedSource(id, source) ?? source.img,
      restored: (id, input, strength, onlyCenter) => {
        const hit = restores.get(id);
        return hit &&
          hit.src === input &&
          hit.strength === strength &&
          hit.onlyCenter === onlyCenter
          ? hit.cv
          : null;
      },
    };
  }

  /** The Viewer's output. Recomputed whenever anything upstream changes. */
  result = $derived.by(() => {
    void this.dataRev;
    const ctx = this.context();
    const cv = evaluate(
      this.viewerId,
      this.nodes,
      this.edges,
      this.params,
      ctx,
    );
    return { cv: cv ?? blank(ctx.w, ctx.h), v: ++version };
  });

  constructor() {
    const id = this.add("viewer", {}, { x: 700, y: 120 });
    this.nodes = this.nodes.map((n) =>
      n.id === id ? { ...n, deletable: false } : n,
    );
    this.selected = null;
  }

  toast(msg: string) {
    this.message = msg;
    setTimeout(() => {
      if (this.message === msg) this.message = "";
    }, 4000);
  }

  add(
    type: string,
    init: Record<string, any> = {},
    position?: { x: number; y: number },
  ) {
    const d = defs[type];
    const id = uid(type);
    const n = this.nodes.length;
    this.params[id] = { visible: true, opacity: 1, ...d.defaults(), ...init };
    // A new image node is a full-frame pass-through when it gets an input.
    if (type === "image") {
      const vp = this.params[this.viewerId];
      if (init.x === undefined) this.params[id].x = (vp?.w ?? 1280) / 2;
      if (init.y === undefined) this.params[id].y = (vp?.h ?? 720) / 2;
    }
    this.nodes = [
      ...this.nodes,
      {
        id,
        type,
        data: {},
        position: position ?? { x: 330, y: 40 + n * 60 },
      },
    ];
    this.selected = id;
    return id;
  }

  /** True for the Group node's input, the only socket that stacks links. */
  inputIsMulti(target: string, handle: string) {
    const node = this.nodes.find((n) => n.id === target);
    const def = node ? defs[node.type ?? ""] : undefined;
    return !!def?.inputs.find((i) => i.id === handle)?.multi;
  }

  /** Drop every link into a single-input socket (used when a new one arrives). */
  clearInput(target: string, handle: string) {
    this.edges = this.edges.filter(
      (e) => !(e.target === target && (e.targetHandle ?? "in") === handle),
    );
  }

  outputIsSingle(source: string) {
    const node = this.nodes.find((n) => n.id === source);
    return !!node && !!defs[node.type ?? ""]?.singleOutput;
  }

  /** Drop every outgoing link from a source (used by the Route node). */
  clearOutput(source: string) {
    this.edges = this.edges.filter((e) => e.source !== source);
  }

  /** Do not stack the exact same source link twice, even on a multi input. */
  clearDuplicate(source: string, target: string, handle: string) {
    this.edges = this.edges.filter(
      (e) =>
        !(
          e.source === source &&
          e.target === target &&
          (e.targetHandle ?? "in") === handle
        ),
    );
  }

  /** True if adding source -> target would close a dependency loop. */
  wouldCycle(source: string, target: string) {
    if (source === target) return true;
    const stack = [target];
    const seen = new Set<string>();
    while (stack.length) {
      const id = stack.pop()!;
      if (id === source) return true;
      if (seen.has(id)) continue;
      seen.add(id);
      for (const e of this.edges) {
        if (e.source === id) stack.push(e.target);
      }
    }
    return false;
  }

  connect(source: string, target: string, handle = "in") {
    if (this.wouldCycle(source, target)) return;
    this.clearDuplicate(source, target, handle);
    if (!this.inputIsMulti(target, handle)) this.clearInput(target, handle);
    if (this.outputIsSingle(source)) this.clearOutput(source);
    this.edges = [
      ...this.edges,
      {
        type: "smoothstep",
        label: "smoothstep",
        id: uid("e"),
        source,
        sourceHandle: "out",
        target,
        targetHandle: handle,
      },
    ];
  }

  /** Add a mix node and wire it into the viewer. */
  addAndConnect(type: string) {
    const id = this.add(type);
    if (defs[type].output) this.connect(id, this.viewerId);
  }

  private async assetFromFile(f: File): Promise<Asset> {
    const url = URL.createObjectURL(f);
    try {
      const img = await load(url);
      return {
        id: uid("a"),
        name: f.name || "pasted-image.png",
        blob: f,
        url,
        img,
        w: img.naturalWidth,
        h: img.naturalHeight,
      };
    } catch (e) {
      URL.revokeObjectURL(url);
      throw e;
    }
  }

  async addFiles(files: File[]) {
    for (const f of files) {
      if (!f.type.startsWith("image/")) continue;
      try {
        const a = await this.assetFromFile(f);
        this.assets = { ...this.assets, [a.id]: a };
        const vp = this.params[this.viewerId];
        const count = this.nodes.filter((n) => n.type === "image").length;
        const id = this.add(
          "image",
          {
            assetId: a.id,
            name: a.name,
            x: vp.w / 2,
            y: vp.h / 2,
            scale: Math.min(1, (vp.w * 0.6) / a.w, (vp.h * 0.6) / a.h),
          },
          { x: 30, y: 30 + count * 250 },
        );
        //Disable auto connect
        // this.connect(id, this.viewerId);
      } catch (e) {
        this.toast((e as Error).message);
      }
    }
  }

  /**
   * Swap the source of an existing image node. Dropping or pasting an image
   * onto a node should not create another layer.
   */
  async replaceImage(id: string, file: File) {
    const node = this.nodes.find((n) => n.id === id);
    const p = this.params[id];
    if (
      !node ||
      node.type !== "image" ||
      !p ||
      !file.type.startsWith("image/")
    ) {
      return;
    }

    try {
      const a = await this.assetFromFile(file);
      const oldAsset = p.assetId;
      this.assets = { ...this.assets, [a.id]: a };
      p.assetId = a.id;
      p.name = a.name;
      p.useMask = false;
      // An upstream image input would take priority over the new asset.
      this.edges = this.edges.filter((e) => e.target !== id);
      masks.delete(id);
      maskKey.delete(id);
      maskVer.delete(id);
      masked.delete(id);

      if (
        oldAsset &&
        oldAsset !== a.id &&
        !Object.values(this.params).some((x) => x?.assetId === oldAsset)
      ) {
        const old = this.assets[oldAsset];
        if (old) {
          URL.revokeObjectURL(old.url);
          const assets = { ...this.assets };
          delete assets[oldAsset];
          this.assets = assets;
        }
      }

      this.dataRev++;
      this.selected = id;
    } catch (e) {
      this.toast((e as Error).message);
    }
  }

  /** Load a file from the served ./input or ./output folder. */
  async addLocal(url: string, name: string) {
    try {
      const r = await fetch(url);
      if (!r.ok) throw new Error(`Could not load ${name}.`);
      const blob = await r.blob();
      const type =
        blob.type ||
        (name.toLowerCase().endsWith(".png") ? "image/png" : "image/jpeg");
      if (!type.startsWith("image/"))
        throw new Error("That file is not an image.");
      await this.addFiles([new File([blob], name, { type })]);
    } catch (e) {
      this.toast((e as Error).message);
    }
  }

  private async fileFromUrl(raw: string) {
    const url = raw.trim();
    const blob = await fetchUrl(url);
    if (!blob.type.startsWith("image/"))
      throw new Error("That URL is not an image.");
    const name = decodeURIComponent(
      new URL(url).pathname.split("/").pop() || "url-image",
    );
    return new File([blob], name, { type: blob.type });
  }

  async addUrl(raw: string) {
    const url = raw.trim();
    if (!/^https?:\/\//i.test(url)) return;
    this.toast("Fetching image…");
    try {
      await this.addFiles([await this.fileFromUrl(url)]);
    } catch (e) {
      this.toast((e as Error).message);
    }
  }

  async replaceUrl(id: string, raw: string) {
    const url = raw.trim();
    if (!/^https?:\/\//i.test(url)) return;
    this.toast("Fetching image…");
    try {
      await this.replaceImage(id, await this.fileFromUrl(url));
    } catch (e) {
      this.toast((e as Error).message);
    }
  }

  /** Incoming evaluated canvas for a node's single image input. */
  inputCv(id: string): Cv | null {
    let incoming: Edge | undefined;
    for (const e of this.edges) {
      if (e.target === id && (e.targetHandle ?? "in") === "in") incoming = e;
    }
    // Evaluate on demand too: an image preview should follow its input even
    // when the image node itself is not connected to the Viewer.
    return incoming ? this.evalNode(incoming.source) : null;
  }

  /** Pixels an image node currently displays: upstream input wins over its asset. */
  imageSource(id: string): ImageSource | null {
    const input = this.inputCv(id);
    if (input)
      return { img: input, w: input.width, h: input.height, key: input };
    const a = this.assets[this.params[id]?.assetId];
    return a ? { img: a.img, w: a.w, h: a.h, key: a.id } : null;
  }

  /** Original file name behind a node's input chain, used for output files. */
  sourceName(id: string): string {
    let cursor = id;
    for (let depth = 0; depth < 32; depth++) {
      const p = this.params[cursor];
      const a = p?.assetId ? this.assets[p.assetId] : undefined;
      if (a?.name) return a.name;
      const incoming = this.edges.find(
        (e) => e.target === cursor && (e.targetHandle ?? "in") === "in",
      );
      if (!incoming) return p?.name || "image";
      cursor = incoming.source;
    }
    return "image";
  }

  /** Does this node have a generated BiRefNet mask? */
  hasMask(id: string) {
    return masks.has(id);
  }

  /** True only when the stored mask belongs to the node's current source. */
  maskReady(id: string) {
    if (!masks.has(id)) return false;
    const source = this.imageSource(id);
    return !!source && maskKey.get(id) === source.key;
  }

  /**
   * The source with its AI mask applied, cached per mask version + source.
   * Returns null when no mask exists or the source changed since it was made.
   * Not reactive by itself: read `graph.dataRev`/`graph.result` beside it.
   */
  maskedSource(id: string, source?: ImageSource): Cv | null {
    const src = source ?? this.imageSource(id);
    const m = masks.get(id);
    if (!src || !m) return null;
    if (maskKey.get(id) !== src.key) return null;
    const v = maskVer.get(id) ?? 0;
    const hit = masked.get(id);
    if (hit?.v === v && hit.key === src.key) return hit.cv;
    const cv = blank(src.w, src.h),
      g = cv.getContext("2d")!;
    g.drawImage(src.img, 0, 0, src.w, src.h);
    g.globalCompositeOperation = "destination-in";
    g.drawImage(m, 0, 0, src.w, src.h);
    masked.set(id, { v, key: src.key, cv });
    return cv;
  }

  /** Generate (or regenerate) the AI mask and enable it on the node. */
  async removeBg(id: string) {
    const p = this.params[id];
    const source = p ? this.imageSource(id) : null;
    if (!p || !source || p.busy) return;
    p.busy = true;
    try {
      const blob = await this.sourceBlob(source, p);
      const result = await mask(blob, this.sourceName(id), 2048);
      // Ignore a result that belongs to an image replaced while we waited.
      const current = this.imageSource(id);
      if (!current || current.key !== source.key) return;
      const u = URL.createObjectURL(result);
      const m = await load(u);
      URL.revokeObjectURL(u);
      const finalSource = this.imageSource(id);
      if (!finalSource || finalSource.key !== source.key) return;
      const cv = blank(m.naturalWidth, m.naturalHeight);
      cv.getContext("2d")!.drawImage(m, 0, 0);
      masks.set(id, cv);
      maskKey.set(id, source.key);
      maskVer.set(id, (maskVer.get(id) ?? 0) + 1);
      masked.delete(id);
      this.dataRev++;
      if (this.params[id]) this.params[id].useMask = true;
    } catch (e) {
      this.toast((e as Error).message);
    } finally {
      if (this.params[id]) this.params[id].busy = false;
    }
  }

  /** The node's current pixels as a PNG blob (canvas or asset). */
  private async sourceBlob(source: ImageSource, p: any): Promise<Blob> {
    const img = source.img;
    if (img instanceof HTMLCanvasElement) {
      return new Promise<Blob>((ok, no) =>
        img.toBlob(
          (b) => (b ? ok(b) : no(new Error("Could not encode image."))),
          "image/png",
        ),
      );
    }
    const a = this.assets[p.assetId];
    if (!a) throw new Error("No image to process.");
    return a.blob;
  }

  /** Restore faces in whatever is currently connected to a Face Restore node. */
  async restoreFaces(id: string) {
    const p = this.params[id];
    if (!p || p.busy) return;
    const input = this.inputCv(id);
    if (!input) {
      this.toast("Connect an image to Face Restore first.");
      return;
    }
    p.busy = true;
    // Show the new result when the run finishes, even if the user was
    // comparing against the original beforehand.
    p.showOriginal = false;
    try {
      const blob = await new Promise<Blob>((ok, no) =>
        input.toBlob(
          (b) => (b ? ok(b) : no(new Error("Could not encode image."))),
          "image/png",
        ),
      );
      const strength = Math.max(0, Math.min(100, Number(p.strength) || 0));
      const result = await restore(
        blob,
        this.sourceName(id),
        strength / 100,
        p.onlyCenter === "center",
      );
      const u = URL.createObjectURL(result);
      const img = await load(u);
      URL.revokeObjectURL(u);
      const cv = blank(img.naturalWidth, img.naturalHeight);
      cv.getContext("2d")!.drawImage(img, 0, 0);
      if (!this.params[id]) return;
      restores.set(id, {
        src: input,
        cv,
        strength,
        onlyCenter: p.onlyCenter === "center",
      });
      restoreVer.set(id, (restoreVer.get(id) ?? 0) + 1);
      this.dataRev++;
    } catch (e) {
      this.toast((e as Error).message);
    } finally {
      if (this.params[id]) this.params[id].busy = false;
    }
  }

  /** Toggle a Face Restore node between its restored and original input. */
  setFaceView(id: string, original: boolean) {
    const p = this.params[id];
    if (p && p.showOriginal !== original) p.showOriginal = original;
  }

  /** "none" = not run yet, "ready" = cache matches input+params, "stale" = re-run. */
  restoreState(id: string): "empty" | "none" | "ready" | "stale" {
    const input = this.inputCv(id);
    if (!input) return "empty";
    const hit = restores.get(id);
    if (!hit) return "none";
    const p = this.params[id];
    const strength = Number(p?.strength ?? 100);
    return hit.src === input &&
      hit.strength === strength &&
      hit.onlyCenter === (p?.onlyCenter === "center")
      ? "ready"
      : "stale";
  }

  /** Drops per-node data. Svelte Flow already removed the nodes/edges itself. */
  cleanup(ids: string[]) {
    for (const id of ids) {
      delete this.params[id];
      masks.delete(id);
      maskKey.delete(id);
      maskVer.delete(id);
      masked.delete(id);
      restores.delete(id);
      restoreVer.delete(id);
      if (this.selected === id) this.selected = null;
    }
  }

  remove(id: string) {
    if (id === this.viewerId) return;
    this.nodes = this.nodes.filter((n) => n.id !== id);
    this.edges = this.edges.filter((e) => e.source !== id && e.target !== id);
    this.cleanup([id]);
  }
  duplicate(id: string) {
    const node = this.nodes.find((n) => n.id === id);
    if (!node || id === this.viewerId) return;

    const newId = uid(node.type as string);
    const params = JSON.parse(JSON.stringify(this.params[id]));

    this.params[newId] = params;

    // Keep the generated mask with the copy so "Use AI mask" keeps working.
    const m = masks.get(id);
    if (m) {
      masks.set(newId, m);
      maskKey.set(newId, maskKey.get(id));
      maskVer.set(newId, maskVer.get(id) ?? 0);
      masked.delete(newId);
      this.dataRev++;
    }

    const duplicate: Node = {
      id: newId,
      type: node.type,
      position: {
        x: node.position.x + 40,
        y: node.position.y + 40,
      },
      data: JSON.parse(JSON.stringify(node.data ?? {})),
      selected: true,
    };

    this.nodes = [
      ...this.nodes.map((n) => (n.id === id ? { ...n, selected: false } : n)),
      duplicate,
    ];

    this.selected = newId;
  }
  /** Layer reorder: stacking order == order of edges into a node. */
  reorder(dragId: string, overId: string, after = false) {
    const a = this.edges.find((e) => e.id === dragId);
    const b = this.edges.find((e) => e.id === overId);
    if (!a || !b || a === b || a.target !== b.target) return;
    const rest = this.edges.filter((e) => e !== a);
    rest.splice(rest.indexOf(b) + (after ? 1 : 0), 0, a);
    this.edges = rest;
  }

  /** Evaluate a single node (and its upstream chain) into the memo cache. */
  evalNode(id: string): Cv | null {
    void this.dataRev;
    return evaluate(id, this.nodes, this.edges, this.params, this.context());
  }

  /** Snapshot everything needed to restore this graph, including processed data. */
  snapshot(): GraphSnapshot {
    const usedAssets = new Set<string>();
    const params: Record<string, any> = {};
    for (const [id, p] of Object.entries(this.params)) {
      if (!p) continue;
      const { busy: _busy, ...rest } = p;
      params[id] = JSON.parse(JSON.stringify(rest));
      if (rest.assetId) usedAssets.add(rest.assetId);
    }
    const live = new Set(this.nodes.map((n) => n.id));
    return {
      nodes: this.nodes.map((n) => ({
        id: n.id,
        type: n.type ?? "image",
        position: { x: n.position.x, y: n.position.y },
        data: JSON.parse(JSON.stringify(n.data ?? {})),
        ...(n.zIndex !== undefined ? { zIndex: n.zIndex } : {}),
        ...(n.draggable !== undefined ? { draggable: n.draggable } : {}),
        ...(n.deletable !== undefined ? { deletable: n.deletable } : {}),
        ...(n.width !== undefined ? { width: n.width } : {}),
        ...(n.height !== undefined ? { height: n.height } : {}),
      })),
      edges: this.edges.map((e) => ({
        id: e.id,
        source: e.source,
        target: e.target,
        sourceHandle: e.sourceHandle,
        targetHandle: e.targetHandle,
      })),
      params,
      assets: Object.values(this.assets).filter((a) => usedAssets.has(a.id)),
      masks: [...masks.entries()]
        .filter(([id]) => live.has(id))
        .map(([nodeId, cv]) => ({ nodeId, cv })),
      restores: [...restores.entries()]
        .filter(([id]) => live.has(id))
        .map(([nodeId, hit]) => ({
          nodeId,
          cv: hit.cv,
          strength: hit.strength,
          onlyCenter: hit.onlyCenter,
        })),
    };
  }

  /**
   * Replace the graph with a saved snapshot + decoded assets/processed canvases.
   * Everything gets fresh ids, then masks/restores are re-keyed to the new
   * source canvases so loading never calls a model.
   */
  importWorkspace(snapshot: GraphData, data: WorkspaceData) {
    for (const asset of Object.values(this.assets)) {
      URL.revokeObjectURL(asset.url);
    }
    clearEvalCache();
    masks.clear();
    maskKey.clear();
    maskVer.clear();
    masked.clear();
    restores.clear();
    restoreVer.clear();

    const assetMap = new Map<string, string>();
    const nextAssets: Record<string, Asset> = {};
    for (const asset of data.assets) {
      const id = uid("a");
      assetMap.set(asset.id, id);
      nextAssets[id] = { ...asset, id };
    }

    const nodeMap = new Map<string, string>();
    const nextNodes: Node[] = [];
    const nextParams: Record<string, any> = {};
    for (const n of snapshot.nodes) {
      const def = defs[n.type];
      if (!def) continue;
      const id = uid(n.type);
      nodeMap.set(n.id, id);
      const saved = (snapshot.params[n.id] ?? {}) as Record<string, any>;
      const p = {
        visible: true,
        opacity: 1,
        ...def.defaults(),
        ...saved,
        busy: false,
      };
      if (p.assetId) p.assetId = assetMap.get(p.assetId) ?? "";
      nextParams[id] = p;
      const node: Node = {
        id,
        type: n.type,
        position: { x: n.position.x, y: n.position.y },
        data: JSON.parse(JSON.stringify(n.data ?? {})),
        ...(n.zIndex !== undefined ? { zIndex: n.zIndex } : {}),
        ...(n.draggable !== undefined ? { draggable: n.draggable } : {}),
        ...(n.deletable !== undefined ? { deletable: n.deletable } : {}),
        ...(n.width !== undefined ? { width: n.width } : {}),
        ...(n.height !== undefined ? { height: n.height } : {}),
      };
      nextNodes.push(node);
    }

    // Exactly one Viewer, always pinned/non-deletable.
    let viewer = nextNodes.find((n) => n.type === "viewer");
    if (!viewer) {
      const id = uid("viewer");
      viewer = {
        id,
        type: "viewer",
        position: { x: 700, y: 120 },
        data: {},
      };
      nextNodes.push(viewer);
      nextParams[id] = {
        visible: true,
        opacity: 1,
        ...defs.viewer.defaults(),
      };
    }
    const kept = new Set<string>();
    for (const node of nextNodes) {
      if (node.type === "viewer" && node !== viewer) continue;
      kept.add(node.id);
      if (node === viewer) {
        node.draggable = true;
        node.deletable = false;
        node.zIndex = 0;
      }
    }
    for (const id of Object.keys(nextParams)) {
      if (!kept.has(id)) delete nextParams[id];
    }

    const nextEdges: Edge[] = [];
    const seenEdges = new Set<string>();
    for (const e of snapshot.edges) {
      const source = nodeMap.get(e.source);
      const target = nodeMap.get(e.target);
      if (!source || !target || !kept.has(source) || !kept.has(target))
        continue;
      const sourceHandle = e.sourceHandle ?? "out";
      const targetHandle = e.targetHandle ?? "in";
      const edgeKey = `${source}:${sourceHandle}:${target}:${targetHandle}`;
      if (seenEdges.has(edgeKey)) continue;
      seenEdges.add(edgeKey);
      nextEdges.push({
        id: uid("e"),
        source,
        target,
        sourceHandle,
        targetHandle,
      });
    }

    // Older templates could stack several links into one socket. Keep only the
    // newest link per single-input socket; Group inputs stay multi.
    const nodeById = new Map(nextNodes.map((n) => [n.id, n]));
    const singleSlot = new Map<string, number>();
    const dedupedEdges: Edge[] = [];
    for (const edge of nextEdges) {
      const node = nodeById.get(edge.target);
      const def = node ? defs[node.type ?? ""] : undefined;
      const handle = edge.targetHandle ?? "in";
      const multi = !!def?.inputs.find((i) => i.id === handle)?.multi;
      if (multi) {
        dedupedEdges.push(edge);
        continue;
      }
      const key = `${edge.target}:${handle}`;
      const at = singleSlot.get(key);
      if (at === undefined) {
        singleSlot.set(key, dedupedEdges.length);
        dedupedEdges.push(edge);
      } else {
        dedupedEdges[at] = edge;
      }
    }

    // Route nodes also keep a single outgoing link when a workspace is loaded.
    const singleOutput = new Map<string, number>();
    const outputDeduped: Edge[] = [];
    for (const edge of dedupedEdges) {
      const node = nodeById.get(edge.source);
      if (!node || !defs[node.type ?? ""]?.singleOutput) {
        outputDeduped.push(edge);
        continue;
      }
      const at = singleOutput.get(edge.source);
      if (at === undefined) {
        singleOutput.set(edge.source, outputDeduped.length);
        outputDeduped.push(edge);
      } else {
        outputDeduped[at] = edge;
      }
    }

    this.nodes = nextNodes.filter((n) => kept.has(n.id));
    this.edges = outputDeduped;
    this.params = nextParams;
    this.assets = nextAssets;
    this.selected = null;
    this.dataRev++;

    const maskEntries = data.masks
      .map((m) => ({ id: nodeMap.get(m.nodeId) ?? "", cv: m.cv }))
      .filter((m) => m.id && kept.has(m.id));
    const restoreEntries = data.restores
      .map((r) => ({ ...r, id: nodeMap.get(r.nodeId) ?? "" }))
      .filter((r) => r.id && kept.has(r.id));

    const sourceFor = (id: string): ImageSource | null => {
      let source = this.imageSource(id);
      if (!source) {
        this.evalNode(id);
        source = this.imageSource(id);
      }
      return source;
    };

    // Masks are keyed by the exact new source. Bump the version whenever the
    // key changes so the evaluator's signature invalidates and applies it.
    for (let pass = 0; pass < 4; pass++) {
      void this.result;
      let changed = false;
      for (const mask of maskEntries) {
        const source = sourceFor(mask.id);
        if (!source || maskKey.get(mask.id) === source.key) continue;
        maskKey.set(mask.id, source.key);
        maskVer.set(mask.id, (maskVer.get(mask.id) ?? 0) + 1);
        changed = true;
      }
      if (!changed) break;
      masked.clear();
      this.dataRev++;
    }

    // Restore entries remember the input canvas they were generated from.
    // Set that only after masks settle, then evaluate once more.
    if (restoreEntries.length) {
      void this.result;
      let changed = false;
      for (const restore of restoreEntries) {
        let input = this.inputCv(restore.id);
        if (!input) {
          this.evalNode(restore.id);
          input = this.inputCv(restore.id);
        }
        if (!input) continue;
        restores.set(restore.id, {
          src: input,
          cv: restore.cv,
          strength: restore.strength,
          onlyCenter: restore.onlyCenter,
        });
        restoreVer.set(restore.id, (restoreVer.get(restore.id) ?? 0) + 1);
        changed = true;
      }
      if (changed) this.dataRev++;
    }
    void this.result;
    this.fitRev++;
  }

  exportPng() {
    this.result.cv.toBlob((b) => {
      if (!b) return;
      const a = document.createElement("a");
      a.href = URL.createObjectURL(b);
      a.download = "composite.png";
      a.click();
    });
  }
}

export const graph = new Graph();
