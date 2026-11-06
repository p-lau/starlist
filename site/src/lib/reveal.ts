import type { Attachment } from 'svelte/attachments';

/** Fades an element in the first time it scrolls into view. */
export const reveal: Attachment<HTMLElement> = (node) => {
	node.classList.add('reveal');
	const io = new IntersectionObserver(
		(entries) => {
			for (const e of entries) {
				if (e.isIntersecting) {
					node.classList.add('in');
					io.disconnect();
				}
			}
		},
		{ rootMargin: '0px 0px -10% 0px' }
	);
	io.observe(node);
	return () => io.disconnect();
};
