<!-- Hero backdrop, as line art: a star seen close up, its edge sweeping across
     the whole width of the screen, with satellites in orbit around it. The
     drawing is cropped to the screen, keeping its bottom edge.

     The satellites are real 3D models (see scene.ts). Each keeps one face to
     the star as it goes round, so from here it turns: front, side, back. It
     grows as it comes toward the viewer and disappears behind the star.

     Four stacked drawings, so that each redraw touches as little as possible:
       back   satellites and orbit lines behind the star (redrawn every frame)
       star   the star itself, with its blurred glow (drawn once, never again)
       grid   the star's turning grid (redrawn about 20 times a second)
       front  satellites and orbit lines in front (redrawn every frame) -->
<script lang="ts">
	import { onMount } from 'svelte';
	import { CX, CY, H, R, W, coronaRays, fleet, orbitTrack, orbits, prominences, satellite, starGrid } from '#lib/scene.ts';

	const SPIN = 0.045; // radians a second
	const tracks = orbits.map(orbitTrack);
	const rays = coronaRays();
	const loops = prominences();

	// The first frame is worked out here, so the server sends a finished picture.
	let grid = $state.raw(starGrid(0));
	const place = (time: number) => fleet.map((f) => ({ ...satellite(f.orbit, f.phase, time), orbit: f.index }));
	let sats = $state.raw(place(0));

	let scene: HTMLElement;

	onMount(() => {
		if (matchMedia('(prefers-reduced-motion: reduce)').matches) return;
		let raf = 0;
		let visible = true;
		let clock = 0; // seconds of animation shown so far
		let last = 0;
		let frame = 0;

		function tick(now: number) {
			raf = 0;
			// A long gap means the tab was hidden: don't jump ahead.
			clock += Math.min((now - (last || now)) / 1000, 0.1);
			last = now;
			sats = place(clock);
			if (frame++ % 3 === 0) grid = starGrid(clock * SPIN);
			if (visible) raf = requestAnimationFrame(tick);
		}

		// Only animate while the scene is on screen.
		const io = new IntersectionObserver(([entry]) => {
			visible = entry.isIntersecting;
			last = 0;
			if (visible && !raf) raf = requestAnimationFrame(tick);
		});
		io.observe(scene);
		return () => {
			io.disconnect();
			cancelAnimationFrame(raf);
		};
	});
</script>

{#snippet craft(front: boolean)}
	{#each sats as sat}
		{#if sat.front === front && sat.parts.length}
			<g class="sat s{sat.orbit}" style="--k:{sat.size.toFixed(2)}">
				{#if sat.link}<path class="link" d={sat.link} />{/if}
				{#each sat.parts as part}
					<path class={part.kind} d={part.d} />
				{/each}
			</g>
		{/if}
	{/each}
{/snippet}

<div class="scene" aria-hidden="true" bind:this={scene}>
	<svg viewBox="0 0 {W} {H}" preserveAspectRatio="xMidYMax slice" focusable="false">
		{#each tracks as track, i}
			<path class="track back t{i}" d={track.back} />
		{/each}
		{@render craft(false)}
	</svg>

	<svg viewBox="0 0 {W} {H}" preserveAspectRatio="xMidYMax slice" focusable="false">
		<defs>
			<radialGradient id="ol-body" gradientUnits="userSpaceOnUse" cx={CX} cy={CY} r={R}>
				<stop offset="0.9" stop-color="#090812" />
				<stop offset="0.975" stop-color="#1c0f0b" />
				<stop offset="1" stop-color="#5a2c07" />
			</radialGradient>
			<!-- Blur only the visible slice: a filter over the whole star would be enormous. -->
			<filter id="ol-halo" filterUnits="userSpaceOnUse" x="-100" y="300" width={W + 200} height={H - 260}>
				<feGaussianBlur stdDeviation="26" />
			</filter>
			<filter id="ol-edge" filterUnits="userSpaceOnUse" x="-100" y="400" width={W + 200} height={H - 360}>
				<feGaussianBlur stdDeviation="4" />
			</filter>
		</defs>
		<circle cx={CX} cy={CY} r={R + 10} fill="none" stroke="#ff8a3d" stroke-width="56" opacity="0.5" filter="url(#ol-halo)" />
		<path class="rays" d={rays} />
		<path class="loops" d={loops} />
		<circle class="disc" cx={CX} cy={CY} r={R} />
		<circle cx={CX} cy={CY} r={R} fill="none" stroke="#ffc857" stroke-width="8" opacity="0.8" filter="url(#ol-edge)" />
		<circle class="limb" cx={CX} cy={CY} r={R} />
		<circle class="limb outer" cx={CX} cy={CY} r={R + 10} />
	</svg>

	<svg viewBox="0 0 {W} {H}" preserveAspectRatio="xMidYMax slice" focusable="false">
		<defs>
			<!-- The grid is brightest toward the edge and fades toward the middle of the star. -->
			<radialGradient id="ol-grid-ink" gradientUnits="userSpaceOnUse" cx={CX} cy={CY} r={R}>
				<stop offset="0.5" stop-color="#ffb04a" stop-opacity="0.12" />
				<stop offset="1" stop-color="#ffd27a" stop-opacity="0.75" />
			</radialGradient>
		</defs>
		<path class="grid" d={grid} />
	</svg>

	<svg viewBox="0 0 {W} {H}" preserveAspectRatio="xMidYMax slice" focusable="false">
		{#each tracks as track, i}
			<path class="track t{i}" d={track.front} />
		{/each}
		{@render craft(true)}
	</svg>
</div>

<style>
	.scene {
		position: absolute;
		inset: 0;
		/* Fade into the page at the bottom, and a little at the very top. */
		mask-image: linear-gradient(transparent, #000 12%, #000 72%, transparent);
	}
	/* On narrow screens a box as tall as the hero would zoom the drawing in
	   until only a sliver of the star showed: keep it short and at the bottom. */
	@media (max-width: 720px) {
		.scene {
			top: auto;
			height: min(150vw, 100%);
			/* the top of this box sits behind the text: fade it out there */
			mask-image: linear-gradient(transparent 18%, #000 52%, #000 72%, transparent);
		}
	}
	svg {
		position: absolute;
		inset: 0;
		width: 100%;
		height: 100%;
		/* its own layer, so redrawing one drawing leaves the others alone */
		will-change: transform;
	}
	path,
	circle {
		fill: none;
		stroke-linecap: round;
		stroke-linejoin: round;
	}
	/* The star's body is opaque, so whatever is behind it is hidden. */
	.disc {
		fill: url(#ol-body);
	}

	.limb {
		stroke: #ffd98a;
		stroke-width: 2.2;
	}
	.limb.outer {
		stroke: #ffb04a;
		stroke-width: 1.1;
		stroke-dasharray: 3 9;
		opacity: 0.7;
	}
	.rays {
		stroke: #ffb04a;
		stroke-width: 1.3;
		opacity: 0.55;
	}
	.loops {
		stroke: #ffc857;
		stroke-width: 1.5;
		opacity: 0.8;
	}
	.grid {
		stroke: url(#ol-grid-ink);
		stroke-width: 1.2;
	}

	.track {
		stroke: var(--orbit);
		stroke-width: 1.3;
		stroke-dasharray: 2 10;
		opacity: 0.75;
	}
	.track.t1 {
		stroke: var(--violet);
	}
	.track.back {
		opacity: 0.35;
	}

	.sat {
		--ink: var(--sat);
		--cells: rgba(79, 108, 255, 0.34);
		stroke: var(--ink);
		stroke-width: calc(var(--k) * 1.6px);
	}
	.sat.s1 {
		--ink: #cdbcff;
		--cells: rgba(140, 104, 255, 0.34);
	}
	.panel {
		fill: var(--cells);
	}
	.body {
		fill: #0b0f2a;
	}
	.dish {
		fill: color-mix(in srgb, var(--ink) 12%, #0b0f2a);
	}
	.link {
		stroke-width: 1.2px;
		stroke-dasharray: 3 8;
		opacity: 0.45;
	}
</style>
