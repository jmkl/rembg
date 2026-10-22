import {
  graph,
  type GraphSnapshot,
  type SnapshotEdge,
  type SnapshotNode,
} from "./graph.svelte";
import { blank, type Asset, type Cv } from "./nodes";

export type WorkspaceKind = "templates" | "projects";

export type WorkspaceMeta = {
  id: string;
  name: string;
  createdAt?: string | null;
  updatedAt?: string | null;
  nodes: number;
  thumb?: string | null;
};

type ManifestAsset = {
  id: string;
  name: string;
  mime: string;
  field?: string;
  file?: string;
};

type ManifestMask = {
  nodeId: string;
  field?: string;
  file?: string;
};

type ManifestRestore = ManifestMask & {
  strength: number;
  onlyCenter: boolean;
};

export type WorkspaceManifest = {
  version?: number;
  id?: string;
  name: string;
  graph: {
    nodes: SnapshotNode[];
    edges: SnapshotEdge[];
    params: Record<string, any>;
  };
  assets?: ManifestAsset[];
  masks?: ManifestMask[];
  restores?: ManifestRestore[];
  thumb?: string | null;
  createdAt?: string | null;
  updatedAt?: string | null;
};

const base = (kind: WorkspaceKind, id?: string) =>
  `/api/workspaces/${kind}${id ? `/${encodeURIComponent(id)}` : ""}`;

function fileUrl(kind: WorkspaceKind, id: string, file: string) {
  const clean = file.replace(/\\/g, "/").replace(/^\/+/, "");
  const path = clean
    .split("/")
    .filter(Boolean)
    .map(encodeURIComponent)
    .join("/");
  // New workspaces reference the shared output pool; old ones kept files in
  // their own folder (assets/, masks/, restores/, thumb.png).
  if (clean.startsWith("output/") || clean.startsWith("workspaces/")) {
    return `/${path}`;
  }
  return `/workspaces/${kind}/${encodeURIComponent(id)}/${path}`;
}

async function errorText(r: Response) {
  try {
    const body = await r.json();
    if (typeof body?.detail === "string") return body.detail;
    if (Array.isArray(body?.detail)) {
      return body.detail.map((d: any) => d?.msg ?? String(d)).join("; ");
    }
  } catch {
    // Not JSON.
  }
  return `Server error ${r.status}`;
}

function loadImage(url: string) {
  return new Promise<HTMLImageElement>((ok, no) => {
    const img = new Image();
    img.onload = () => ok(img);
    img.onerror = () => no(new Error("Could not decode a saved image."));
    img.src = url;
  });
}

export async function decodeAsset(
  id: string,
  name: string,
  blob: Blob,
): Promise<Asset> {
  const url = URL.createObjectURL(blob);
  try {
    const img = await loadImage(url);
    return {
      id,
      name,
      blob,
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

export async function decodeCanvas(blob: Blob): Promise<Cv> {
  const url = URL.createObjectURL(blob);
  try {
    const img = await loadImage(url);
    const cv = blank(img.naturalWidth, img.naturalHeight);
    cv.getContext("2d")!.drawImage(img, 0, 0);
    return cv;
  } finally {
    URL.revokeObjectURL(url);
  }
}

export function encodePng(cv: Cv): Promise<Blob> {
  return new Promise((ok, no) =>
    cv.toBlob(
      (blob) => (blob ? ok(blob) : no(new Error("Could not encode image."))),
      "image/png",
    ),
  );
}

/** Small preview drawn from the current Viewer result. */
async function makeThumb(cv: Cv): Promise<Blob> {
  const scale = Math.min(1, 320 / Math.max(1, cv.width));
  const width = Math.max(1, Math.round(cv.width * scale));
  const height = Math.max(1, Math.round(cv.height * scale));
  const out = blank(width, height);
  const g = out.getContext("2d")!;
  g.imageSmoothingQuality = "high";
  g.drawImage(cv, 0, 0, width, height);
  return encodePng(out);
}

export async function listWorkspaces(
  kind: WorkspaceKind,
): Promise<WorkspaceMeta[]> {
  const r = await fetch(base(kind));
  if (!r.ok) throw new Error(await errorText(r));
  return (await r.json()) as WorkspaceMeta[];
}

/**
 * Upload the current graph, all referenced source images, AI masks, face
 * restores and a thumbnail. `id` overwrites an existing project in place.
 */
export async function saveWorkspace(
  kind: WorkspaceKind,
  name: string,
  id?: string,
): Promise<WorkspaceMeta> {
  const snap: GraphSnapshot = graph.snapshot();
  const manifest: WorkspaceManifest = {
    version: 1,
    name,
    graph: { nodes: snap.nodes, edges: snap.edges, params: snap.params },
    assets: [],
    masks: [],
    restores: [],
  };
  const form = new FormData();

  for (let i = 0; i < snap.assets.length; i++) {
    const asset = snap.assets[i];
    const field = `asset_${i}`;
    manifest.assets!.push({
      id: asset.id,
      name: asset.name,
      mime: asset.blob.type || "image/png",
      field,
    });
    form.append(field, asset.blob, asset.name || `image-${i}.png`);
  }

  for (let i = 0; i < snap.masks.length; i++) {
    const mask = snap.masks[i];
    const field = `mask_${i}`;
    manifest.masks!.push({ nodeId: mask.nodeId, field });
    form.append(field, await encodePng(mask.cv), `${mask.nodeId}.png`);
  }

  for (let i = 0; i < snap.restores.length; i++) {
    const restore = snap.restores[i];
    const field = `restore_${i}`;
    manifest.restores!.push({
      nodeId: restore.nodeId,
      strength: restore.strength,
      onlyCenter: restore.onlyCenter,
      field,
    });
    form.append(
      field,
      await encodePng(restore.cv),
      `${restore.nodeId}.png`,
    );
  }

  form.append("thumb", await makeThumb(graph.result.cv), "thumb.png");
  form.append("manifest", JSON.stringify(manifest));

  const r = await fetch(base(kind, id), {
    method: id ? "PUT" : "POST",
    body: form,
  });
  if (!r.ok) throw new Error(await errorText(r));
  return (await r.json()) as WorkspaceMeta;
}

/**
 * Load a saved graph and its images/processed canvases, then make it current.
 * No inference requests are made: masks and restores are read from disk.
 */
export async function loadWorkspace(
  kind: WorkspaceKind,
  id: string,
): Promise<WorkspaceManifest> {
  const r = await fetch(base(kind, id));
  if (!r.ok) throw new Error(await errorText(r));
  const manifest = (await r.json()) as WorkspaceManifest;
  if (!manifest?.graph?.nodes || !manifest?.graph?.edges) {
    throw new Error("That saved workspace is damaged.");
  }

  const assets: Asset[] = [];
  try {
    for (const item of manifest.assets ?? []) {
      if (!item.file) continue;
      const file = await fetch(fileUrl(kind, id, item.file));
      if (!file.ok) throw new Error(`Missing saved image: ${item.name}`);
      assets.push(await decodeAsset(item.id, item.name, await file.blob()));
    }

    const masks: { nodeId: string; cv: Cv }[] = [];
    for (const item of manifest.masks ?? []) {
      if (!item.file) continue;
      const file = await fetch(fileUrl(kind, id, item.file));
      if (!file.ok) throw new Error("Missing a saved AI mask.");
      masks.push({ nodeId: item.nodeId, cv: await decodeCanvas(await file.blob()) });
    }

    const restores: {
      nodeId: string;
      cv: Cv;
      strength: number;
      onlyCenter: boolean;
    }[] = [];
    for (const item of manifest.restores ?? []) {
      if (!item.file) continue;
      const file = await fetch(fileUrl(kind, id, item.file));
      if (!file.ok) throw new Error("Missing a saved face restore.");
      restores.push({
        nodeId: item.nodeId,
        cv: await decodeCanvas(await file.blob()),
        strength: Number(item.strength ?? 100),
        onlyCenter: !!item.onlyCenter,
      });
    }

    graph.importWorkspace(manifest.graph, { assets, masks, restores });
  } catch (e) {
    for (const asset of assets) URL.revokeObjectURL(asset.url);
    throw e;
  }

  return manifest;
}

export async function renameWorkspace(
  kind: WorkspaceKind,
  id: string,
  name: string,
): Promise<WorkspaceMeta> {
  const r = await fetch(base(kind, id), {
    method: "PATCH",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ name }),
  });
  if (!r.ok) throw new Error(await errorText(r));
  return (await r.json()) as WorkspaceMeta;
}

export async function deleteWorkspace(kind: WorkspaceKind, id: string) {
  const r = await fetch(base(kind, id), { method: "DELETE" });
  if (!r.ok) throw new Error(await errorText(r));
}
