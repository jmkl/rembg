import {
  graph,
  type SnapshotEdge,
  type SnapshotNode,
} from "./graph.svelte";
import type { Asset, Cv } from "./nodes";
import { decodeAsset, decodeCanvas, encodePng } from "./workspace";

export type SessionIdentity = {
  project: { id: string; name: string } | null;
  template: { id: string; name: string } | null;
};

type StoredAsset = {
  id: string;
  name: string;
  mime: string;
  blob: Blob;
};

type StoredMask = {
  nodeId: string;
  blob: Blob;
};

type StoredRestore = StoredMask & {
  strength: number;
  onlyCenter: boolean;
};

type SessionRecord = {
  version: 1;
  savedAt: number;
  graph: {
    nodes: SnapshotNode[];
    edges: SnapshotEdge[];
    params: Record<string, any>;
  };
  assets: StoredAsset[];
  masks: StoredMask[];
  restores: StoredRestore[];
  project: { id: string; name: string } | null;
  template: { id: string; name: string } | null;
};

const DB_NAME = "rmbg-studio-session";
const STORE = "state";
const KEY = "current";

function openDb(): Promise<IDBDatabase> {
  return new Promise((ok, no) => {
    if (typeof indexedDB === "undefined") {
      no(new Error("IndexedDB is not available."));
      return;
    }
    const req = indexedDB.open(DB_NAME, 1);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains(STORE)) db.createObjectStore(STORE);
    };
    req.onsuccess = () => ok(req.result);
    req.onerror = () =>
      no(req.error ?? new Error("Could not open session storage."));
    req.onblocked = () => no(new Error("Session storage is blocked."));
  });
}

async function putRecord(record: SessionRecord): Promise<void> {
  const db = await openDb();
  try {
    await new Promise<void>((ok, no) => {
      const tx = db.transaction(STORE, "readwrite");
      tx.objectStore(STORE).put(record, KEY);
      tx.oncomplete = () => ok();
      tx.onerror = () =>
        no(tx.error ?? new Error("Could not save the session."));
      tx.onabort = () =>
        no(tx.error ?? new Error("Saving the session was aborted."));
    });
  } finally {
    db.close();
  }
}

async function getRecord(): Promise<SessionRecord | undefined> {
  const db = await openDb();
  try {
    return await new Promise<SessionRecord | undefined>((ok, no) => {
      const tx = db.transaction(STORE, "readonly");
      const req = tx.objectStore(STORE).get(KEY);
      req.onsuccess = () => ok(req.result as SessionRecord | undefined);
      req.onerror = () =>
        no(req.error ?? new Error("Could not read the session."));
    });
  } finally {
    db.close();
  }
}

async function deleteRecord(): Promise<void> {
  const db = await openDb();
  try {
    await new Promise<void>((ok, no) => {
      const tx = db.transaction(STORE, "readwrite");
      tx.objectStore(STORE).delete(KEY);
      tx.oncomplete = () => ok();
      tx.onerror = () =>
        no(tx.error ?? new Error("Could not delete the session."));
      tx.onabort = () =>
        no(tx.error ?? new Error("Deleting the session was aborted."));
    });
  } finally {
    db.close();
  }
}

// Masks/restore canvases only change when the user re-runs a model or edits
// them, so reuse their encoded PNG across autosaves.
const pngCache = new WeakMap<Cv, Promise<Blob>>();

function encodePngCached(cv: Cv): Promise<Blob> {
  let blob = pngCache.get(cv);
  if (!blob) {
    blob = encodePng(cv);
    pngCache.set(cv, blob);
    blob.catch(() => pngCache.delete(cv));
  }
  return blob;
}

/** Persist the whole current graph and its decoded images in IndexedDB. */
export async function saveSession(identity: SessionIdentity): Promise<void> {
  const snap = graph.snapshot();
  const record: SessionRecord = {
    version: 1,
    savedAt: Date.now(),
    graph: { nodes: snap.nodes, edges: snap.edges, params: snap.params },
    assets: snap.assets.map((asset) => ({
      id: asset.id,
      name: asset.name,
      mime: asset.blob.type || "image/png",
      blob: asset.blob,
    })),
    masks: [],
    restores: [],
    project: identity.project ? { ...identity.project } : null,
    template: identity.template ? { ...identity.template } : null,
  };

  for (const mask of snap.masks) {
    record.masks.push({
      nodeId: mask.nodeId,
      blob: await encodePngCached(mask.cv),
    });
  }
  for (const restore of snap.restores) {
    record.restores.push({
      nodeId: restore.nodeId,
      strength: restore.strength,
      onlyCenter: restore.onlyCenter,
      blob: await encodePngCached(restore.cv),
    });
  }

  await putRecord(record);
}

/**
 * Restore the last autosaved session without calling the backend. Returns the
 * project/template identity so Ctrl+S can keep targeting the same save.
 */
export async function loadSession(): Promise<SessionIdentity | null> {
  const record = await getRecord();
  if (!record || record.version !== 1 || !record.graph?.nodes) return null;

  const identity = (value: any) =>
    value && typeof value.id === "string" && typeof value.name === "string"
      ? { id: value.id, name: value.name }
      : null;

  const assets: Asset[] = [];
  try {
    for (const item of record.assets ?? []) {
      assets.push(await decodeAsset(item.id, item.name, item.blob));
    }

    const masks: { nodeId: string; cv: Cv }[] = [];
    for (const item of record.masks ?? []) {
      masks.push({
        nodeId: item.nodeId,
        cv: await decodeCanvas(item.blob),
      });
    }

    const restores: {
      nodeId: string;
      cv: Cv;
      strength: number;
      onlyCenter: boolean;
    }[] = [];
    for (const item of record.restores ?? []) {
      restores.push({
        nodeId: item.nodeId,
        cv: await decodeCanvas(item.blob),
        strength: Number(item.strength ?? 100),
        onlyCenter: !!item.onlyCenter,
      });
    }

    graph.importWorkspace(record.graph, { assets, masks, restores });
  } catch (e) {
    for (const asset of assets) URL.revokeObjectURL(asset.url);
    throw e;
  }

  return {
    project: identity(record.project),
    template: identity(record.template),
  };
}

/** Called after Save as project/template: the current state is no longer a draft. */
export async function clearSession(): Promise<void> {
  await deleteRecord();
}
