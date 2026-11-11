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
		// A shooting star now and then: where it starts, its direction, when it began.
		let streak: { x: number; y: number; dx: number; dy: number; t0: number } | undefined;
		let nextStreak = 2500;
		let lastFrame = -1000;

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
			// Twinkling doesn't need 60 frames a second; about 30 halves the work.
			if (!still && t - lastFrame < 30) {
				raf = requestAnimationFrame(draw);
				return;
			}
			lastFrame = t;
			ctx!.clearRect(0, 0, w, h);
			for (const st of stars) {
				const twinkle = still ? 1 : 0.65 + 0.35 * Math.sin(t * 0.001 * st.s + st.p);
				ctx!.globalAlpha = st.a * twinkle;
				ctx!.fillStyle = `hsl(${st.hue} 90% 88%)`;
				ctx!.beginPath();
				ctx!.arc(st.x, st.y, st.r, 0, Math.PI * 2);
				ctx!.fill();
			}
			if (!still) {
				drawStreak(t);
				raf = requestAnimationFrame(draw);
			}
		}

		function drawStreak(t: number) {
			if (!streak && t > nextStreak) {
				const angle = Math.PI * (0.12 + Math.random() * 0.14);
				const len = 260 + Math.random() * 220;
				streak = {
					x: Math.random() * w * 0.8,
					y: Math.random() * h * 0.45,
					dx: Math.cos(angle) * len,
					dy: Math.sin(angle) * len,
					t0: t
				};
			}
			if (!streak) return;
			const k = (t - streak.t0) / 900;
			if (k >= 1) {
				streak = undefined;
				nextStreak = t + 5000 + Math.random() * 7000;
				return;
			}
			const head = 1 - (1 - k) ** 3;
			const tail = Math.max(0, head - 0.28);
			const x1 = streak.x + streak.dx * tail;
			const y1 = streak.y + streak.dy * tail;
			const x2 = streak.x + streak.dx * head;
			const y2 = streak.y + streak.dy * head;
			const g = ctx!.createLinearGradient(x1, y1, x2, y2);
			g.addColorStop(0, 'rgba(159, 231, 255, 0)');
			g.addColorStop(1, 'rgba(255, 246, 216, 0.9)');
			ctx!.globalAlpha = Math.sin(Math.PI * k);
			ctx!.strokeStyle = g;
			ctx!.lineWidth = 1.4;
			ctx!.lineCap = 'round';
			ctx!.beginPath();
			ctx!.moveTo(x1, y1);
			ctx!.lineTo(x2, y2);
			ctx!.stroke();
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
