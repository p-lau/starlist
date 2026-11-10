<!-- Hero illustration: the app is a star, the collector is a satellite that
     picks up raw log noise on one side and beams a small summary out the other.

     The orbits are real 3D: each ring is a tilted plane, an arm turns inside it,
     and the satellite at the end of the arm is turned back by exactly the same
     amount (.pivot undoes the spin, .sat undoes the tilt). So a satellite always
     faces the viewer, passes behind the star on the far side and in front of it
     on the near side, and looks a little larger when it is close.

     The visible orbit lines are drawn separately (.tracks, flattened into the
     star's plane). The rings that carry the satellites paint nothing: a painted
     tilted plane would cut through a satellite and the browser would clip it. -->
<div class="scene" aria-hidden="true">
	<div class="glow"></div>
	<div class="star"></div>

	<div class="tracks">
		<div class="ring r1"></div>
		<div class="ring r2"></div>
		<div class="ring r3"></div>
	</div>

	<div class="ring carrier r1">
		<div class="arm"><div class="pivot"><div class="sat"><i></i><i></i><b></b></div></div></div>
	</div>
	<div class="ring carrier r2">
		<div class="arm"><div class="pivot"><div class="sat small"><i></i><i></i><b></b></div></div></div>
	</div>

	<div class="noise">
		{#each Array(14) as _, i}
			<i style="--i:{i}"></i>
		{/each}
	</div>
</div>

<style>
	.scene {
		position: relative;
		width: min(460px, 86vw);
		aspect-ratio: 1;
		margin: 0 auto;
		perspective: 1100px;
		transform-style: preserve-3d;
	}

	.glow {
		position: absolute;
		inset: 18%;
		border-radius: 50%;
		background: radial-gradient(circle, rgba(255, 200, 87, 0.5), rgba(255, 138, 61, 0.12) 45%, transparent 70%);
		filter: blur(10px);
		animation: pulse 6s ease-in-out infinite;
	}

	.star {
		position: absolute;
		inset: 38%;
		border-radius: 50%;
		background: radial-gradient(circle at 38% 35%, #fffaf0, #ffd77a 35%, #ff9a3d 75%, #d9541a);
		box-shadow:
			0 0 40px 10px rgba(255, 180, 70, 0.55),
			0 0 120px 40px rgba(255, 138, 61, 0.25);
	}

	.ring {
		--pitch: 68deg;
		position: absolute;
		border-radius: 50%;
		border: 1px solid rgba(122, 162, 255, 0.28);
		top: 50%;
		left: 50%;
		transform: translate(-50%, -50%) rotateX(var(--pitch)) rotateZ(var(--tilt));
		transform-style: preserve-3d;
	}

	.tracks {
		position: absolute;
		inset: 0;
		perspective: 1100px;
	}
	.carrier {
		border-color: transparent !important;
	}

	.r1 {
		width: 70%;
		height: 70%;
		--tilt: -20deg;
		--period: 14s;
	}
	.r2 {
		width: 92%;
		height: 92%;
		--tilt: 30deg;
		--period: 26s;
		--dir: reverse;
		border-style: dashed;
		border-color: rgba(182, 156, 255, 0.22);
	}
	.r3 {
		width: 112%;
		height: 112%;
		--tilt: 5deg;
		border-color: rgba(122, 162, 255, 0.1);
	}

	.arm {
		position: absolute;
		inset: 0;
		transform-style: preserve-3d;
		animation: spin var(--period) linear infinite var(--dir, normal);
	}
	.pivot {
		position: absolute;
		top: 0;
		left: 50%;
		width: 0;
		height: 0;
		transform-style: preserve-3d;
		transform: rotateZ(calc(-1 * var(--tilt)));
		animation: unspin var(--period) linear infinite var(--dir, normal);
	}

	/* The satellite: a body, two solar panels and a dish, leaning a little. */
	.sat {
		--s: 14px;
		--c: var(--sat);
		--panel: linear-gradient(90deg, #3d59e8, #8fd4ff);
		position: absolute;
		width: var(--s);
		height: var(--s);
		margin: calc(var(--s) / -2);
		border-radius: 3px;
		background: linear-gradient(135deg, #fff, var(--c) 55%);
		box-shadow: 0 0 14px 4px color-mix(in srgb, var(--c) 55%, transparent);
		transform: rotateX(calc(-1 * var(--pitch))) rotate(-16deg);
	}
	.sat i {
		position: absolute;
		top: 22%;
		width: 115%;
		height: 56%;
		border-radius: 1.5px;
		/* panel colour with thin dark lines between the cells */
		background:
			repeating-linear-gradient(90deg, transparent 0 3px, rgba(5, 6, 15, 0.55) 3px 4px),
			var(--panel);
	}
	.sat i:nth-child(1) {
		right: 118%;
	}
	.sat i:nth-child(2) {
		left: 118%;
	}
	.sat b {
		position: absolute;
		left: 50%;
		bottom: 100%;
		width: 36%;
		height: 36%;
		margin-left: -18%;
		border-radius: 50% 50% 0 0;
		background: #fff;
		opacity: 0.9;
	}
	.sat.small {
		--s: 10px;
		--c: var(--violet);
		--panel: linear-gradient(90deg, #6a4df0, #d7c9ff);
	}

	/* Raw log "noise" drifting in toward the star */
	.noise i {
		position: absolute;
		left: calc(4% + var(--i) * 6.5%);
		top: calc(50% + (var(--i) - 7) * 1.5%);
		width: 2px;
		height: 2px;
		border-radius: 50%;
		background: rgba(232, 235, 255, 0.5);
		animation: drift 5s linear infinite;
		animation-delay: calc(var(--i) * -0.37s);
		opacity: 0;
	}

	@keyframes spin {
		to {
			transform: rotateZ(1turn);
		}
	}
	@keyframes unspin {
		to {
			transform: rotateZ(calc(-1 * var(--tilt) - 1turn));
		}
	}
	@keyframes pulse {
		50% {
			transform: scale(1.08);
			opacity: 0.85;
		}
	}
	@keyframes drift {
		0% {
			transform: translate(-40px, 0);
			opacity: 0;
		}
		30% {
			opacity: 0.7;
		}
		100% {
			transform: translate(120px, 0);
			opacity: 0;
		}
	}
</style>
