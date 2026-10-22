import type { Edge, Node } from '@xyflow/svelte'
import { blank, defs, type Ctx, type Cv } from './nodes'

const cache = new Map<string, { sig: string; cv: Cv | null }>()

/** Last computed output of a node (for thumbnails). */
export const getOut = (id: string) => cache.get(id)?.cv ?? null

/** Drop memoized node outputs (used when a saved workspace replaces the graph). */
export const clearEvalCache = () => cache.clear()

/**
 * Pull-based evaluation from `root` upstream. Each node is cached by a
 * signature of (params, input signatures, output size, external revision),
 * so dragging one image only re-renders that node and what is downstream.
 */
export function evaluate(
  root: string, nodes: Node[], edges: Edge[], params: Record<string, any>, ctx: Ctx,
): Cv | null {
  const ev = (id: string, path: string[]): { cv: Cv | null; sig: string } => {
    const node = nodes.find((n) => n.id === id)
    const def = node && defs[node.type ?? '']
    const p = params[id]
    if (!def || !p || path.includes(id)) return { cv: null, sig: 'x' }

    const inputs: Record<string, Cv[]> = {}
    const sigs: string[] = []
    for (const s of def.inputs) {
      inputs[s.id] = []
      for (const e of edges) {
        if (e.target !== id || (e.targetHandle ?? s.id) !== s.id) continue
        const r = ev(e.source, [...path, id])
        sigs.push(r.sig)
        if (r.cv) inputs[s.id].push(r.cv)
      }
    }

    const sig = JSON.stringify([p, sigs, ctx.w, ctx.h, ctx.rev(id)])
    const hit = cache.get(id)
    if (hit?.sig === sig) return { cv: hit.cv, sig }

    let cv = p.visible === false ? null : def.evaluate(ctx, p, inputs, id)
    if (cv && p.opacity < 1) {
      const o = blank(ctx.w, ctx.h), g = o.getContext('2d')!
      g.globalAlpha = p.opacity
      g.drawImage(cv, 0, 0)
      cv = o
    }
    cache.set(id, { sig, cv })
    return { cv, sig }
  }
  return ev(root, []).cv
}
