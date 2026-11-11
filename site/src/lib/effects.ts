import type { Attachment } from 'svelte/attachments';

const canHover = () => matchMedia('(hover: hover) and (pointer: fine)').matches;
const still = () => matchMedia('(prefers-reduced-motion: reduce)').matches;

/**
 * Lights up whichever `.spot` element is under the pointer by setting --mx and
 * --my on it. One listener for the whole container, at most one write a frame.
 */
export const spotlight: Attachment<HTMLElement> = (node) => {
	if (!canHover()) return;
	let frame = 0;
	let last: PointerEvent | undefined;

	function apply() {
		frame = 0;
		const target = (last?.target as Element | null)?.closest<HTMLElement>('.spot');
		if (!target || !last) return;
		const box = target.getBoundingClientRect();
		target.style.setProperty('--mx', `${last.clientX - box.left}px`);
		target.style.setProperty('--my', `${last.clientY - box.top}px`);
	}
	function move(e: PointerEvent) {
		last = e;
		frame ||= requestAnimationFrame(apply);
	}

	node.addEventListener('pointermove', move, { passive: true });
	return () => {
		node.removeEventListener('pointermove', move);
		cancelAnimationFrame(frame);
	};
};

/** Leans an element a few degrees toward the pointer. */
export const tilt: Attachment<HTMLElement> = (node) => {
	if (!canHover() || still()) return;
	const max = 3.5;
	let frame = 0;
	let last: PointerEvent | undefined;

	function apply() {
		frame = 0;
		if (!last) return;
		const box = node.getBoundingClientRect();
		const x = (last.clientX - box.left) / box.width - 0.5;
		const y = (last.clientY - box.top) / box.height - 0.5;
		node.style.setProperty('--ry', `${(x * max * 2).toFixed(2)}deg`);
		node.style.setProperty('--rx', `${(-y * max * 2).toFixed(2)}deg`);
	}
	function move(e: PointerEvent) {
		last = e;
		frame ||= requestAnimationFrame(apply);
	}
	function leave() {
		last = undefined;
		cancelAnimationFrame(frame);
		frame = 0;
		node.style.setProperty('--rx', '0deg');
		node.style.setProperty('--ry', '0deg');
	}

	node.addEventListener('pointermove', move, { passive: true });
	node.addEventListener('pointerleave', leave);
	return () => {
		node.removeEventListener('pointermove', move);
		node.removeEventListener('pointerleave', leave);
		cancelAnimationFrame(frame);
	};
};
