// Transition helpers shared by every component, so entrances and exits feel alike.
import { fly, fade, scale, slide } from 'svelte/transition';
import { cubicOut } from 'svelte/easing';
import type { TransitionConfig } from 'svelte/transition';

const reduce = typeof matchMedia !== 'undefined' && matchMedia('(prefers-reduced-motion: reduce)').matches;
/** Duration in ms, zero when the person prefers reduced motion. */
export const d = (ms: number) => (reduce ? 0 : ms);

type Node = Element;

/** A screen or block arriving: slides up a little while fading in. */
export const enter = (node: Node, p: { y?: number; x?: number; duration?: number; delay?: number } = {}): TransitionConfig =>
  fly(node, { y: 12, duration: d(260), easing: cubicOut, ...p, ...(p.duration != null ? { duration: d(p.duration) } : {}) });

/** Moving between questions or steps: slides in from the side it is heading to. */
export const sweep = (node: Node, p: { dir?: 1 | -1 } = {}): TransitionConfig => fly(node, { x: 36 * (p.dir ?? 1), duration: d(260), easing: cubicOut });

/** A floating panel (popover): grows out of its corner. */
export const pop = (node: Node): TransitionConfig => ({
  duration: d(170),
  easing: cubicOut,
  css: (t) => `opacity:${t};transform:translateY(${(1 - t) * -6}px) scale(${0.96 + 0.04 * t});transform-origin:top right`,
});

/** A dropdown list: drops down a few pixels. */
export const drop = (node: Node): TransitionConfig => fly(node, { y: -6, duration: d(150), easing: cubicOut });

/** Something leaving or arriving in a list. */
export const item = (node: Node): TransitionConfig => fly(node, { x: -24, duration: d(200), easing: cubicOut });

/** Small things appearing (thumbnails, pills). */
export const grow = (node: Node): TransitionConfig => scale(node, { start: 0.85, duration: d(180), easing: cubicOut });

/** Height-and-fade expansion for things that appear in the flow (feedback boxes). */
export const open = (node: Node): TransitionConfig => slide(node, { duration: d(220), easing: cubicOut });

export const softFade = (node: Node): TransitionConfig => fade(node, { duration: d(160) });
export const flipDur = () => d(240);
