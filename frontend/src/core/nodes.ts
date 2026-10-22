/**
 * NODE REGISTRY — the only file you touch to add a node type.
 *
 * A node = sockets + default params + evaluate(). Every socket carries a
 * canvas image, and `inputs[socketId]` is always an array ordered bottom -> top
 * (edge order), so single- and multi-input nodes share one code path.
 *
 * The UI is generic: NodeView draws handles from `inputs`, a form from
 * `fields`, and the Layers panel / transform box work off `transform`.
 * Opacity + visibility are applied by the evaluator for every node.
 */
export type Cv = HTMLCanvasElement;

export interface Asset {
  id: string;
  name: string;
  blob: Blob;
  url: string;
  img: HTMLImageElement;
  w: number;
  h: number;
}

/** An image that can receive an AI mask: an asset or an upstream canvas. */
export interface ImageSource {
  img: CanvasImageSource;
  w: number;
  h: number;
  /** Identity of the exact pixels, so masks can't be reused on a different source. */
  key: unknown;
}

export interface Ctx {
  w: number;
  h: number;
  assets: Record<string, Asset>;
  /** Source with the AI mask applied (cached by the graph store). */
  mask: (nodeId: string, source: ImageSource) => CanvasImageSource;
  /** Cached face-restore output, or null when missing/stale. */
  restored: (
    nodeId: string,
    input: Cv,
    strength: number,
    onlyCenter: boolean,
  ) => Cv | null;
  /** Bumps whenever a node's external data (mask/restore) changes; part of cache key. */
  rev: (nodeId: string) => number;
}

export interface Field {
  key: string;
  label: string;
  type: "range" | "number" | "color" | "select";
  min?: number;
  max?: number;
  step?: number;
  options?: { label: string; value: string }[];
}

export interface NodeDef<P = any> {
  type: string;
  title: string;
  color: string;
  inputs: { id: string; multi?: boolean }[];
  output: boolean;
  /** Only one outgoing link may exist (used by the Route node). */
  singleOutput?: boolean;
  /** Shown in the header "add" buttons (image/viewer have their own flow). */
  creatable?: boolean;
  /** Has x / y / scale / rotation params -> movable in Edit mode. */
  transform?: boolean;
  fields?: Field[];
  defaults: () => P;
  evaluate: (
    c: Ctx,
    p: P,
    inputs: Record<string, Cv[]>,
    id: string,
  ) => Cv | null;
}

export function blank(w: number, h: number): Cv {
  const c = document.createElement("canvas");
  c.width = w;
  c.height = h;
  return c;
}

const stack = (c: Ctx, list: Cv[]) => {
  const out = blank(c.w, c.h),
    g = out.getContext("2d")!;
  for (const cv of list) g.drawImage(cv, 0, 0);
  return out;
};

const clamp = (v: number, lo: number, hi: number) =>
  Math.max(lo, Math.min(hi, v));
const clamp01 = (v: number) => clamp(v, 0, 1);

/** Draw an input into the Viewer-sized frame and expose its pixels. */
function pixels(c: Ctx, input: Cv) {
  const out = blank(c.w, c.h);
  const g = out.getContext("2d", { willReadFrequently: true })!;
  g.drawImage(input, 0, 0, c.w, c.h);
  const img = g.getImageData(0, 0, c.w, c.h);
  return { out, g, img };
}

export type CurvePoint = { x: number; y: number };

export const identityCurve = (): CurvePoint[] => [
  { x: 0, y: 0 },
  { x: 1, y: 1 },
];

/** Normalize saved curve data into sorted, clamped points with fixed ends. */
export function sanitizeCurve(points: unknown): CurvePoint[] {
  const list = Array.isArray(points) ? points : [];
  const out: CurvePoint[] = [];
  for (const raw of list) {
    if (!raw || typeof raw !== "object") continue;
    const x = Number((raw as CurvePoint).x);
    const y = Number((raw as CurvePoint).y);
    if (!Number.isFinite(x) || !Number.isFinite(y)) continue;
    out.push({ x: clamp01(x), y: clamp01(y) });
  }
  out.sort((a, b) => a.x - b.x);
  const unique: CurvePoint[] = [];
  for (const point of out) {
    if (
      unique.length &&
      Math.abs(unique[unique.length - 1].x - point.x) < 1e-6
    ) {
      unique[unique.length - 1] = point;
    } else {
      unique.push(point);
    }
  }
  if (unique.length < 2) return identityCurve();
  if (unique[0].x > 1e-6) unique.unshift({ x: 0, y: unique[0].y });
  if (unique[unique.length - 1].x < 1 - 1e-6) {
    unique.push({ x: 1, y: unique[unique.length - 1].y });
  }
  unique[0].x = 0;
  unique[unique.length - 1].x = 1;
  return unique;
}

/** Monotone cubic interpolation, matching Photoshop's smooth curve feel. */
export function curveSampler(points: unknown): (x: number) => number {
  const pts = sanitizeCurve(points);
  const n = pts.length;
  if (n === 2) {
    const a = pts[0];
    const b = pts[1];
    return (x: number) => {
      if (x <= a.x) return a.y;
      if (x >= b.x) return b.y;
      return clamp01(a.y + ((b.y - a.y) * (x - a.x)) / (b.x - a.x));
    };
  }

  const dx: number[] = [];
  const slope: number[] = [];
  const tangent: number[] = [];
  for (let i = 0; i < n - 1; i++) {
    dx[i] = Math.max(1e-6, pts[i + 1].x - pts[i].x);
    slope[i] = (pts[i + 1].y - pts[i].y) / dx[i];
  }
  tangent[0] = slope[0];
  tangent[n - 1] = slope[n - 2];
  for (let i = 1; i < n - 1; i++) {
    tangent[i] = (slope[i - 1] + slope[i]) / 2;
  }
  for (let i = 0; i < n - 1; i++) {
    if (Math.abs(slope[i]) < 1e-8) {
      tangent[i] = 0;
      tangent[i + 1] = 0;
      continue;
    }
    const a = tangent[i] / slope[i];
    const b = tangent[i + 1] / slope[i];
    const h = Math.hypot(a, b);
    if (h > 3) {
      const t = 3 / h;
      tangent[i] = t * a * slope[i];
      tangent[i + 1] = t * b * slope[i];
    }
  }

  return (x: number) => {
    x = clamp01(x);
    if (x <= pts[0].x) return pts[0].y;
    if (x >= pts[n - 1].x) return pts[n - 1].y;
    let i = 0;
    while (i < n - 2 && x > pts[i + 1].x) i++;
    const t = (x - pts[i].x) / dx[i];
    const t2 = t * t;
    const t3 = t2 * t;
    const y =
      (2 * t3 - 3 * t2 + 1) * pts[i].y +
      (t3 - 2 * t2 + t) * dx[i] * tangent[i] +
      (-2 * t3 + 3 * t2) * pts[i + 1].y +
      (t3 - t2) * dx[i] * tangent[i + 1];
    return clamp01(y);
  };
}

/** 256-entry lookup table shared by the evaluator. */
export function curveLut(points: unknown): Uint8ClampedArray {
  const sample = curveSampler(points);
  const lut = new Uint8ClampedArray(256);
  for (let i = 0; i < 256; i++) {
    lut[i] = Math.round(sample(i / 255) * 255);
  }
  return lut;
}

function hexRgb(value: unknown): [number, number, number] {
  const match = /^#?([0-9a-f]{6})$/i.exec(String(value ?? "").trim());
  if (!match) return [0, 0, 0];
  const n = parseInt(match[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

// Small, fast value-noise helpers for the Noise Texture node.
function hash2(ix: number, iy: number, seed: number) {
  let h =
    Math.imul(ix, 374761393) +
    Math.imul(iy, 668265263) +
    Math.imul(seed, 1442695041);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
}

function valueNoise(x: number, y: number, seed: number) {
  const xi = Math.floor(x);
  const yi = Math.floor(y);
  const xf = x - xi;
  const yf = y - yi;
  const u = xf * xf * (3 - 2 * xf);
  const v = yf * yf * (3 - 2 * yf);
  const a = hash2(xi, yi, seed);
  const b = hash2(xi + 1, yi, seed);
  const c = hash2(xi, yi + 1, seed);
  const d = hash2(xi + 1, yi + 1, seed);
  return a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v;
}

const image: NodeDef = {
  type: "image",
  title: "Image",
  color: "#a63c47",
  output: true,
  transform: true,
  inputs: [{ id: "in" }],
  defaults: () => ({
    assetId: "",
    x: 640,
    y: 360,
    scale: 1,
    rotation: 0,
    useMask: false,
    busy: false,
  }),
  evaluate: (c, p, i, id) => {
    const input = i.in?.[0];
    const a = c.assets[p.assetId];
    if (!input && !a) return null;

    const w = input ? input.width : a!.w;
    const h = input ? input.height : a!.h;
    // With an input the upstream canvas becomes the source, so a newly added
    // image node is a pass-through until it is moved/scaled.
    const source: ImageSource = input
      ? { img: input, w, h, key: input }
      : { img: a!.img, w: a!.w, h: a!.h, key: a!.id };

    const out = blank(c.w, c.h),
      g = out.getContext("2d")!;
    g.imageSmoothingQuality = "high";
    g.translate(p.x, p.y);
    g.rotate((p.rotation * Math.PI) / 180);
    g.scale(p.scale, p.scale);
    g.drawImage(
      (p.useMask ? c.mask(id, source) : null) ?? source.img,
      -w / 2,
      -h / 2,
      w,
      h,
    );
    return out;
  },
};

// Stored as type "mix" for compatibility with saved graphs; the UI calls it Group.
const mix: NodeDef = {
  type: "mix",
  title: "Group",
  color: "#4a7c59",
  output: true,
  creatable: true,
  inputs: [{ id: "in", multi: true }],
  defaults: () => ({}),
  evaluate: (c, _p, i) => stack(c, i.in),
};

const viewer: NodeDef = {
  type: "viewer",
  title: "Viewer",
  color: "#a34a3f",
  output: false,
  inputs: [{ id: "in", multi: true }],
  fields: [
    { key: "w", label: "Width", type: "number", min: 16, max: 4096 },
    { key: "h", label: "Height", type: "number", min: 16, max: 4096 },
  ],
  defaults: () => ({ w: 1280, h: 720 }),
  evaluate: (c, _p, i) => stack(c, i.in),
};

/** Example effect node: ~10 lines, no Svelte code. Curves etc. follow this shape. */
const hsv: NodeDef = {
  type: "hsv",
  title: "Hue / Saturation",
  color: "#7c5cbf",
  output: true,
  creatable: true,
  inputs: [{ id: "in" }],
  fields: [
    { key: "hue", label: "Hue", type: "range", min: 0, max: 360 },
    { key: "sat", label: "Saturation", type: "range", min: 0, max: 300 },
    { key: "val", label: "Brightness", type: "range", min: 0, max: 200 },
  ],
  defaults: () => ({ hue: 0, sat: 100, val: 100 }),
  evaluate: (c, p, i) => {
    if (!i.in[0]) return null;
    const out = blank(c.w, c.h),
      g = out.getContext("2d")!;
    g.filter = `hue-rotate(${p.hue}deg) saturate(${p.sat}%) brightness(${p.val}%)`;
    g.drawImage(i.in[0], 0, 0);
    return out;
  },
};

const curves: NodeDef = {
  type: "curves",
  title: "Curves",
  color: "#2563eb",
  output: true,
  creatable: true,
  inputs: [{ id: "in" }],
  defaults: () => ({
    channel: "rgb",
    curves: {
      rgb: identityCurve(),
      red: identityCurve(),
      green: identityCurve(),
      blue: identityCurve(),
    },
  }),
  evaluate: (c, p, i) => {
    const input = i.in?.[0];
    if (!input) return null;
    const curves = p.curves ?? {};
    const master = curveLut(curves.rgb);
    const red = curveLut(curves.red);
    const green = curveLut(curves.green);
    const blue = curveLut(curves.blue);
    const { out, g, img } = pixels(c, input);
    const d = img.data;
    for (let k = 0; k < d.length; k += 4) {
      d[k] = red[master[d[k]]];
      d[k + 1] = green[master[d[k + 1]]];
      d[k + 2] = blue[master[d[k + 2]]];
    }
    g.putImageData(img, 0, 0);
    return out;
  },
};

const exposure: NodeDef = {
  type: "exposure",
  title: "Exposure",
  color: "#ca8a04",
  output: true,
  creatable: true,
  inputs: [{ id: "in" }],
  fields: [
    {
      key: "ev",
      label: "Exposure (EV)",
      type: "range",
      min: -5,
      max: 5,
      step: 0.01,
    },
    {
      key: "offset",
      label: "Offset (%)",
      type: "range",
      min: -50,
      max: 50,
      step: 1,
    },
    {
      key: "gamma",
      label: "Gamma",
      type: "range",
      min: 0.1,
      max: 3,
      step: 0.01,
    },
  ],
  defaults: () => ({ ev: 0, offset: 0, gamma: 1 }),
  evaluate: (c, p, i) => {
    const input = i.in?.[0];
    if (!input) return null;
    const ev = clamp(Number(p.ev ?? 0), -5, 5);
    const offset = clamp(Number(p.offset ?? 0), -50, 50) / 100;
    const gamma = clamp(Number(p.gamma ?? 1), 0.1, 3);
    const gain = Math.pow(2, ev);
    const lut = new Uint8ClampedArray(256);
    for (let v = 0; v < 256; v++) {
      const n = clamp01((v / 255) * gain + offset);
      lut[v] = Math.round(Math.pow(n, 1 / gamma) * 255);
    }
    const { out, g, img } = pixels(c, input);
    const d = img.data;
    for (let k = 0; k < d.length; k += 4) {
      d[k] = lut[d[k]];
      d[k + 1] = lut[d[k + 1]];
      d[k + 2] = lut[d[k + 2]];
    }
    g.putImageData(img, 0, 0);
    return out;
  },
};

const colorbalance: NodeDef = {
  type: "colorbalance",
  title: "Color Balance",
  color: "#0d9488",
  output: true,
  creatable: true,
  inputs: [{ id: "in" }],
  fields: [
    {
      key: "range",
      label: "Tonal Range",
      type: "select",
      options: [
        { label: "Shadows", value: "shadows" },
        { label: "Midtones", value: "midtones" },
        { label: "Highlights", value: "highlights" },
      ],
    },
    {
      key: "cyanRed",
      label: "Cyan / Red",
      type: "range",
      min: -100,
      max: 100,
      step: 1,
    },
    {
      key: "magentaGreen",
      label: "Magenta / Green",
      type: "range",
      min: -100,
      max: 100,
      step: 1,
    },
    {
      key: "yellowBlue",
      label: "Yellow / Blue",
      type: "range",
      min: -100,
      max: 100,
      step: 1,
    },
    {
      key: "preserve",
      label: "Preserve Luminosity",
      type: "select",
      options: [
        { label: "Yes", value: "yes" },
        { label: "No", value: "no" },
      ],
    },
  ],
  defaults: () => ({
    range: "midtones",
    cyanRed: 0,
    magentaGreen: 0,
    yellowBlue: 0,
    preserve: "yes",
  }),
  evaluate: (c, p, i) => {
    const input = i.in?.[0];
    if (!input) return null;
    const range = p.range ?? "midtones";
    const cr = clamp(Number(p.cyanRed ?? 0), -100, 100) / 100;
    const mg = clamp(Number(p.magentaGreen ?? 0), -100, 100) / 100;
    const yb = clamp(Number(p.yellowBlue ?? 0), -100, 100) / 100;
    const preserve = p.preserve !== "no";
    const { out, g, img } = pixels(c, input);
    const d = img.data;
    for (let k = 0; k < d.length; k += 4) {
      const r = d[k] / 255;
      const green = d[k + 1] / 255;
      const b = d[k + 2] / 255;
      const lum = 0.2126 * r + 0.7152 * green + 0.0722 * b;
      const w =
        range === "shadows"
          ? (1 - lum) * (1 - lum)
          : range === "highlights"
            ? lum * lum
            : 4 * lum * (1 - lum);
      let nr = r + cr * 0.5 * w;
      let ng = green + mg * 0.5 * w;
      let nb = b + yb * 0.5 * w;
      if (preserve) {
        const nextLum = 0.2126 * nr + 0.7152 * ng + 0.0722 * nb;
        if (nextLum > 0.0001) {
          const scale = lum / nextLum;
          nr *= scale;
          ng *= scale;
          nb *= scale;
        }
      }
      d[k] = clamp01(nr) * 255;
      d[k + 1] = clamp01(ng) * 255;
      d[k + 2] = clamp01(nb) * 255;
    }
    g.putImageData(img, 0, 0);
    return out;
  },
};

const gradientmap: NodeDef = {
  type: "gradientmap",
  title: "Gradient Map",
  color: "#7c3aed",
  output: true,
  creatable: true,
  inputs: [{ id: "in" }],
  fields: [
    { key: "color1", label: "Shadow Color", type: "color" },
    { key: "color2", label: "Mid Color", type: "color" },
    { key: "color3", label: "Highlight Color", type: "color" },
    {
      key: "midpoint",
      label: "Midpoint (%)",
      type: "range",
      min: 0,
      max: 100,
      step: 1,
    },
    {
      key: "reverse",
      label: "Reverse",
      type: "select",
      options: [
        { label: "No", value: "no" },
        { label: "Yes", value: "yes" },
      ],
    },
  ],
  defaults: () => ({
    color1: "#000000",
    color2: "#808080",
    color3: "#ffffff",
    midpoint: 50,
    reverse: "no",
  }),
  evaluate: (c, p, i) => {
    const input = i.in?.[0];
    if (!input) return null;
    const pos = clamp(Number(p.midpoint ?? 50), 0, 100) / 100;
    const reverse = p.reverse === "yes";
    const shadow = hexRgb(p.color1);
    const mid = hexRgb(p.color2);
    const highlight = hexRgb(p.color3);
    const lut = new Uint8ClampedArray(256 * 3);
    for (let v = 0; v < 256; v++) {
      let t = v / 255;
      if (reverse) t = 1 - t;
      const [a, b, local] =
        t <= pos
          ? ([shadow, mid, pos <= 0 ? 0 : t / pos] as const)
          : ([mid, highlight, pos >= 1 ? 0 : (t - pos) / (1 - pos)] as const);
      lut[v * 3] = a[0] + (b[0] - a[0]) * local;
      lut[v * 3 + 1] = a[1] + (b[1] - a[1]) * local;
      lut[v * 3 + 2] = a[2] + (b[2] - a[2]) * local;
    }
    const { out, g, img } = pixels(c, input);
    const d = img.data;
    for (let k = 0; k < d.length; k += 4) {
      const lum = 0.2126 * d[k] + 0.7152 * d[k + 1] + 0.0722 * d[k + 2];
      const idx = Math.round(lum) * 3;
      d[k] = lut[idx];
      d[k + 1] = lut[idx + 1];
      d[k + 2] = lut[idx + 2];
    }
    g.putImageData(img, 0, 0);
    return out;
  },
};

const invert: NodeDef = {
  type: "invert",
  title: "Invert",
  color: "#64748b",
  output: true,
  creatable: true,
  inputs: [{ id: "in" }],
  fields: [
    {
      key: "amount",
      label: "Amount",
      type: "range",
      min: 0,
      max: 100,
      step: 1,
    },
  ],
  defaults: () => ({ amount: 100 }),
  evaluate: (c, p, i) => {
    const input = i.in?.[0];
    if (!input) return null;
    const amount = clamp(Number(p.amount ?? 100), 0, 100) / 100;
    const { out, g, img } = pixels(c, input);
    const d = img.data;
    for (let k = 0; k < d.length; k += 4) {
      d[k] = d[k] + (255 - d[k] * 2) * amount;
      d[k + 1] = d[k + 1] + (255 - d[k + 1] * 2) * amount;
      d[k + 2] = d[k + 2] + (255 - d[k + 2] * 2) * amount;
    }
    g.putImageData(img, 0, 0);
    return out;
  },
};

const gradient: NodeDef = {
  type: "gradient",
  title: "Gradient",
  color: "#d97706",
  output: true,
  creatable: true,
  inputs: [],

  fields: [
    {
      key: "mode",
      label: "Type",
      type: "select",
      options: [
        { label: "Linear", value: "linear" },
        { label: "Radial", value: "radial" },
        { label: "Solid Color", value: "solid" },
      ],
    },
    { key: "color1", label: "Color 1", type: "color" },
    { key: "color2", label: "Color 2", type: "color" },
    { key: "angle", label: "Angle", type: "range", step: 5, min: 0, max: 360 },
    {
      key: "color1Position",
      label: "Color 1 Position (%)",
      type: "range",
      min: -200,
      max: 200,
    },
    {
      key: "color2Position",
      label: "Color 2 Position (%)",
      type: "range",
      min: -200,
      max: 200,
    },
    {
      key: "width",
      label: "Width (px)",
      type: "number",
      min: 100,
      max: 2000,
      step: 1,
    },
    {
      key: "height",
      label: "Height (px)",
      type: "number",
      min: 100,
      max: 2000,
      step: 1,
    },
  ],

  defaults: () => ({
    mode: "linear",
    color1: "#000000",
    color2: "#ffffff",
    angle: 90,
    split: 50,
    width: 1280,
    height: 720,
  }),

  evaluate: (c, p) => {
    const w = Math.max(100, Math.min(2000, p.width ?? 1920));
    const h = Math.max(100, Math.min(2000, p.height ?? 1080));

    const out = blank(w, h);
    const g = out.getContext("2d")!;

    if (p.mode === "solid") {
      g.fillStyle = p.color1;
      g.fillRect(0, 0, w, h);
      return out;
    }

    let gr: CanvasGradient;

    if (p.mode === "radial") {
      gr = g.createRadialGradient(
        w / 2,
        h / 2,
        0,
        w / 2,
        h / 2,
        Math.hypot(w, h) / 2,
      );
    } else {
      const a = ((p.angle ?? 90) * Math.PI) / 180;
      const dx = (Math.cos(a) * w) / 2;
      const dy = (Math.sin(a) * h) / 2;

      gr = g.createLinearGradient(
        w / 2 - dx,
        h / 2 - dy,
        w / 2 + dx,
        h / 2 + dy,
      );
    }

    const normalize = (value: number) =>
      Math.max(0, Math.min(1, (value + 200) / 400));

    const pos1 = normalize(p.color1Position);
    const pos2 = normalize(p.color2Position);

    gr.addColorStop(0, p.color1);
    gr.addColorStop(Math.min(pos1, pos2), p.color1);
    gr.addColorStop(Math.max(pos1, pos2), p.color2);
    gr.addColorStop(1, p.color2);

    g.fillStyle = gr;
    g.fillRect(0, 0, w, h);

    return out;
  },
};

const noise: NodeDef = {
  type: "noise",
  title: "Noise Texture",
  color: "#8a5a2b",
  output: true,
  creatable: true,
  inputs: [],
  fields: [
    {
      key: "kind",
      label: "Noise Type",
      type: "select",
      options: [
        { label: "fBM", value: "fbm" },
        { label: "Multifractal", value: "multifractal" },
        { label: "Ridged", value: "ridged" },
      ],
    },
    { key: "scale", label: "Scale", type: "range", min: 0.1, max: 30, step: 0.1 },
    { key: "detail", label: "Detail", type: "range", min: 0, max: 10, step: 1 },
    {
      key: "roughness",
      label: "Roughness",
      type: "range",
      min: 0,
      max: 1,
      step: 0.01,
    },
    {
      key: "lacunarity",
      label: "Lacunarity",
      type: "range",
      min: 1,
      max: 10,
      step: 0.1,
    },
    {
      key: "distortion",
      label: "Distortion",
      type: "range",
      min: 0,
      max: 5,
      step: 0.1,
    },
    { key: "seed", label: "Seed", type: "number", min: 0, max: 9999, step: 1 },
  ],
  defaults: () => ({
    kind: "fbm",
    scale: 5,
    detail: 4,
    roughness: 0.5,
    lacunarity: 2,
    distortion: 0,
    seed: 0,
  }),
  evaluate: (c, p) => {
    // Output size follows the Viewer, so the noise is resolution-independent.
    const w = Math.max(1, c.w);
    const h = Math.max(1, c.h);
    const scale = Math.max(0.01, Number(p.scale ?? 5));
    const octaves = Math.max(
      1,
      Math.min(8, Math.round(Number(p.detail ?? 4))),
    );
    const roughness = Math.max(0, Math.min(1, Number(p.roughness ?? 0.5)));
    const lacunarity = Math.max(1, Math.min(10, Number(p.lacunarity ?? 2)));
    const distortion = Math.max(0, Number(p.distortion ?? 0));
    const seed = Math.round(Number(p.seed ?? 0));
    const kind = p.kind ?? "fbm";
    const step = scale / Math.min(w, h);

    const out = blank(w, h);
    const g = out.getContext("2d")!;
    const img = g.createImageData(w, h);
    const data = img.data;

    for (let y = 0; y < h; y++) {
      for (let x = 0; x < w; x++) {
        let nx = x * step;
        let ny = y * step;

        if (distortion > 0) {
          const amount = distortion * 0.15;
          const dx =
            (valueNoise(nx * 0.7 + 11.3, ny * 0.7 - 7.9, seed + 101) - 0.5) *
            amount;
          const dy =
            (valueNoise(nx * 0.7 - 3.1, ny * 0.7 + 5.7, seed + 211) - 0.5) *
            amount;
          nx += dx;
          ny += dy;
        }

        let sum = 0;
        let amp = 1;
        let freq = 1;
        let norm = 0;

        for (let o = 0; o < octaves; o++) {
          let n = valueNoise(nx * freq, ny * freq, seed + o * 173);
          if (kind === "ridged") n = 1 - Math.abs(n * 2 - 1);
          else if (kind === "multifractal") n *= 0.5 + 0.5 * n;
          sum += n * amp;
          norm += amp;
          amp *= roughness;
          freq *= lacunarity;
        }

        let v = norm > 0 ? sum / norm : 0;
        v = v * v * (3 - 2 * v);

        const i = (y * w + x) * 4;
        const gray = Math.round(Math.max(0, Math.min(1, v)) * 255);
        data[i] = gray;
        data[i + 1] = gray;
        data[i + 2] = gray;
        data[i + 3] = 255;
      }
    }

    g.putImageData(img, 0, 0);
    return out;
  },
};
const route: NodeDef = {
  type: "route",
  title: "Route",
  color: "#5f6b7a",
  output: true,
  creatable: true,
  singleOutput: true,
  inputs: [{ id: "in" }],
  defaults: () => ({}),
  evaluate: (_c, _p, i) => i.in?.[0] ?? null,
};

const blend: NodeDef = {
  type: "blend",
  title: "Blend",
  color: "#8b5cf6",
  output: true,
  creatable: true,

  inputs: [{ id: "base" }, { id: "blend" }],

  fields: [
    {
      key: "mode",
      label: "Blend Mode",
      type: "select",
      options: [
        { label: "Normal", value: "source-over" },
        { label: "Multiply", value: "multiply" },
        { label: "Screen", value: "screen" },
        { label: "Overlay", value: "overlay" },
        { label: "Darken", value: "darken" },
        { label: "Lighten", value: "lighten" },
        { label: "Color Dodge", value: "color-dodge" },
        { label: "Color Burn", value: "color-burn" },
        { label: "Hard Light", value: "hard-light" },
        { label: "Soft Light", value: "soft-light" },
        { label: "Difference", value: "difference" },
        { label: "Exclusion", value: "exclusion" },
        { label: "Hue", value: "hue" },
        { label: "Saturation", value: "saturation" },
        { label: "Color", value: "color" },
        { label: "Luminosity", value: "luminosity" },
      ],
    },
    {
      key: "opacity",
      label: "Opacity",
      type: "range",
      min: 0,
      max: 100,
      step: 1,
    },
  ],

  defaults: () => ({
    mode: "source-over",
    opacity: 100,
  }),

  evaluate: (c, p, inputs) => {
    const base = inputs.base?.[0];
    const blend = inputs.blend?.[0];

    const out = blank(
      base?.width ?? blend?.width ?? c.w,
      base?.height ?? blend?.height ?? c.h,
    );

    const ctx = out.getContext("2d")!;

    if (base) {
      ctx.drawImage(base, 0, 0, out.width, out.height);
    }

    if (blend) {
      ctx.globalCompositeOperation = p.mode ?? "source-over";
      ctx.globalAlpha = Math.max(0, Math.min(100, p.opacity ?? 100)) / 100;

      ctx.drawImage(blend, 0, 0, out.width, out.height);
    }

    ctx.globalAlpha = 1;
    ctx.globalCompositeOperation = "source-over";

    return out;
  },
};

const mask: NodeDef = {
  type: "mask",
  title: "Mask",
  color: "#8b5cf6",
  output: true,
  creatable: true,

  inputs: [{ id: "image" }, { id: "mask" }],

  fields: [
    {
      key: "invert",
      label: "Invert Mask",
      type: "select",
      options: [
        { label: "Dark = Opaque", value: "dark" },
        { label: "Light = Opaque", value: "light" },
      ],
    },
    {
      key: "strength",
      label: "Strength",
      type: "range",
      min: 0,
      max: 100,
    },
  ],

  defaults: () => ({
    invert: "dark",
    strength: 100,
    opacity: 1,
    visible: true,
  }),

  evaluate: (c, p, inputs) => {
    const image = inputs.image?.[0];
    const mask = inputs.mask?.[0];

    if (!image) return blank(c.w, c.h);

    const out = blank(c.w, c.h);
    const g = out.getContext("2d")!;

    // Draw the image first.
    g.drawImage(image, 0, 0, c.w, c.h);

    if (!mask) return out;

    // Read mask brightness from its rendered pixels.
    const maskCanvas = blank(c.w, c.h);
    const mg = maskCanvas.getContext("2d", {
      willReadFrequently: true,
    })!;

    mg.drawImage(mask, 0, 0, c.w, c.h);

    const maskData = mg.getImageData(0, 0, c.w, c.h);
    const imageData = g.getImageData(0, 0, c.w, c.h);

    const strength = Math.max(0, Math.min(100, p.strength ?? 100)) / 100;
    const darkIsOpaque = p.invert !== "light";

    for (let i = 0; i < maskData.data.length; i += 4) {
      const r = maskData.data[i];
      const green = maskData.data[i + 1];
      const b = maskData.data[i + 2];

      // Perceived brightness, from 0 (black) to 1 (white).
      const brightness = (0.2126 * r + 0.7152 * green + 0.0722 * b) / 255;

      const maskAlpha = darkIsOpaque ? 1 - brightness : brightness;

      const factor = 1 - strength + maskAlpha * strength;

      imageData.data[i + 3] *= factor;
    }

    g.putImageData(imageData, 0, 0);

    return out;
  },
};

const facerestore: NodeDef = {
  type: "facerestore",
  title: "Face Restore",
  color: "#c2410c",
  output: true,
  creatable: true,
  inputs: [{ id: "in" }],
  fields: [
    { key: "strength", label: "Strength", type: "range", min: 0, max: 100 },
    {
      key: "onlyCenter",
      label: "Faces",
      type: "select",
      options: [
        { label: "All faces", value: "all" },
        { label: "Center face only", value: "center" },
      ],
    },
  ],
  defaults: () => ({
    strength: 100,
    onlyCenter: "all",
    showOriginal: false,
    busy: false,
  }),
  evaluate: (c, p, i, id) => {
    const input = i.in?.[0];
    if (!input) return null;
    if (p.showOriginal) return input;
    // While no result exists (or the input/params changed) pass the frame
    // through so the rest of the graph still previews something sensible.
    return (
      c.restored(id, input, p.strength ?? 100, p.onlyCenter === "center") ??
      input
    );
  },
};

export const defs: Record<string, NodeDef> = {
  image,
  route,
  blend,
  mask,
  mix,
  viewer,
  hsv,
  curves,
  exposure,
  colorbalance,
  gradientmap,
  invert,
  gradient,
  noise,
  facerestore,
};
