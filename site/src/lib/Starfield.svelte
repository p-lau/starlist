<script lang="ts">
	import { onMount } from 'svelte';

	let canvas: HTMLCanvasElement;

	onMount(() => {
		const ctx = canvas.getContext('2d');
		if (!ctx) return;

		const still = matchMedia('(prefers-reduced-motion: reduce)').matches;
		let w = 0;
		let h = 0;
		let raf = 0;
		let stars: { x: number; y: number; r: number; a: number; s: number; p: number; hue: number }[] =
			[];

		function resize() {
			const dpr = Math.min(devicePixelRatio || 1, 2);
			w = canvas.clientWidth;
			h = canvas.clientHeight;
			canvas.width = w * dpr;
			canvas.height = h * dpr;
			ctx!.setTransform(dpr, 0, 0, dpr, 0, 0);
			const count = Math.round((w * h) / 2600);
			stars = Array.from({ length: count }, () => ({
				x: Math.random() * w,
				y: Math.random() * h,
				r: Math.random() ** 3 * 1.6 + 0.25,
				a: Math.random() * 0.7 + 0.2,
				s: Math.random() * 0.8 + 0.2,
				p: Math.random() * Math.PI * 2,
				hue: Math.random() < 0.15 ? 40 : Math.random() < 0.3 ? 200 : 230
			}));
		}

		function draw(t: number) {
			ctx!.clearRect(0, 0, w, h);
			for (const st of stars) {
				const twinkle = still ? 1 : 0.65 + 0.35 * Math.sin(t * 0.001 * st.s + st.p);
				ctx!.globalAlpha = st.a * twinkle;
				ctx!.fillStyle = `hsl(${st.hue} 90% 88%)`;
				ctx!.beginPath();
				ctx!.arc(st.x, st.y, st.r, 0, Math.PI * 2);
				ctx!.fill();
			}
			if (!still) raf = requestAnimationFrame(draw);
		}

		resize();
		draw(0);
		const ro = new ResizeObserver(() => {
			resize();
			if (still) draw(0);
		});
		ro.observe(canvas);
		return () => {
			cancelAnimationFrame(raf);
			ro.disconnect();
		};
	});
</script>

<canvas bind:this={canvas} aria-hidden="true"></canvas>

<style>
	canvas {
		position: fixed;
		inset: 0;
		width: 100%;
		height: 100%;
		pointer-events: none;
		z-index: 0;
	}
</style>
