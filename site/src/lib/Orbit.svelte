<!-- Hero illustration: the app is a star, the collector is a satellite that
     picks up raw log noise on one side and beams a small summary out the other. -->
<div class="scene" aria-hidden="true">
	<div class="glow"></div>
	<div class="star"></div>

	<div class="ring r1"><div class="sat"><span></span></div></div>
	<div class="ring r2"><div class="sat small"><span></span></div></div>
	<div class="ring r3"></div>

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
		position: absolute;
		border-radius: 50%;
		border: 1px solid rgba(122, 162, 255, 0.28);
		top: 50%;
		left: 50%;
		transform: translate(-50%, -50%) rotateX(68deg) rotateZ(var(--tilt));
		transform-style: preserve-3d;
	}

	.r1 {
		width: 70%;
		height: 70%;
		--tilt: -20deg;
		animation: spin 14s linear infinite;
	}
	.r2 {
		width: 92%;
		height: 92%;
		--tilt: 30deg;
		border-style: dashed;
		border-color: rgba(182, 156, 255, 0.22);
		animation: spin 26s linear infinite reverse;
	}
	.r3 {
		width: 112%;
		height: 112%;
		--tilt: 5deg;
		border-color: rgba(122, 162, 255, 0.1);
	}

	.sat {
		position: absolute;
		top: -7px;
		left: 50%;
		width: 14px;
		height: 14px;
		margin-left: -7px;
		border-radius: 3px;
		background: var(--sat);
		box-shadow: 0 0 14px 4px rgba(159, 231, 255, 0.6);
		transform: rotateX(-68deg);
	}
	.sat span {
		position: absolute;
		top: 5px;
		left: -12px;
		width: 38px;
		height: 4px;
		border-radius: 2px;
		background: linear-gradient(90deg, #4f6cff, #9fe7ff 40%, #9fe7ff 60%, #4f6cff);
		opacity: 0.85;
	}
	.sat.small {
		width: 10px;
		height: 10px;
		margin-left: -5px;
		top: -5px;
		background: var(--violet);
		box-shadow: 0 0 12px 3px rgba(182, 156, 255, 0.55);
	}
	.sat.small span {
		top: 3px;
		left: -9px;
		width: 28px;
		height: 3px;
		background: linear-gradient(90deg, #7a5cff, #d7c9ff 50%, #7a5cff);
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
			transform: translate(-50%, -50%) rotateX(68deg) rotateZ(calc(var(--tilt) + 360deg));
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
