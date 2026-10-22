import type { Cv } from './nodes'

/** use:paint={canvas} — copies a canvas into the element (re-runs on change). */
export function paint(el: HTMLCanvasElement, cv?: Cv | null) {
  const draw = (c?: Cv | null) => {
    if (!c) { el.width = el.height = 0; return }
    el.width = c.width; el.height = c.height
    el.getContext('2d')!.drawImage(c, 0, 0)
  }
  draw(cv)
  return { update: draw }
}
