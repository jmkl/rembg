// Backend API seam. Every call goes through this module so the transport
// (the desktop custom protocol, or a local server in tests) stays swappable.
// Images travel as base64 in JSON bodies; binary responses stay binary.

export type FolderFile = {
  name: string;
  kind: "input" | "output";
  url: string;
  size: number;
  mtime: number;
};

async function postImage(path: string, body: unknown): Promise<Blob> {
  const r = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!r.ok) {
    throw new Error(
      (await r.json().catch(() => ({}))).detail || `Server error ${r.status}`,
    );
  }
  return r.blob();
}

function blobToDataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result as string);
    reader.onerror = () => reject(new Error("Could not read the image."));
    reader.readAsDataURL(blob);
  });
}

/** BiRefNet soft alpha mask (white RGB + mask alpha). */
export async function mask(
  image: Blob,
  name: string,
  maxSide = 2048,
): Promise<Blob> {
  return postImage("/api/mask", {
    image: await blobToDataUrl(image),
    name,
    max_side: maxSide,
  });
}

/** GFPGAN face restoration. `strength` is 0..1. */
export async function restore(
  image: Blob,
  name: string,
  strength: number,
  onlyCenter: boolean,
): Promise<Blob> {
  return postImage("/api/restore", {
    image: await blobToDataUrl(image),
    name,
    strength,
    only_center: onlyCenter,
  });
}

/** Download a URL server-side and re-encode it for a reliable preview. */
export async function fetchUrl(url: string): Promise<Blob> {
  const r = await fetch("/api/fetch?url=" + encodeURIComponent(url));
  if (!r.ok) {
    throw new Error(
      (await r.json().catch(() => ({}))).detail || "Could not load that URL.",
    );
  }
  return r.blob();
}

export async function listFiles(): Promise<{
  input: FolderFile[];
  output: FolderFile[];
}> {
  const r = await fetch("/api/files");
  if (!r.ok) throw new Error(`Server error ${r.status}`);
  return r.json();
}
