<!-- Hero illustration: the app is a star, the collector is a satellite that
     picks up raw log noise on one side and beams a small summary out the other.

     Everything is drawn as SVG with filters: noise for the star's surface and
     the satellites' foil, lighting to give that noise relief, displacement to
     pull the corona into wisps, and blur for glow.

     Filters are costly to draw, so nothing filtered ever changes. Each layer is
     drawn once and then only turned, moved or faded as a whole by its wrapper,
     which the compositor does without redrawing.

     The orbits are real 3D: each ring is a tilted plane, an arm turns inside it,
     and the satellite at the end of the arm is turned back by exactly the same
     amount (.pivot undoes the spin, .sat undoes the tilt). So a satellite always
     faces the viewer, passes behind the star on the far side and in front of it
     on the near side, and looks a little larger when it is close.

     The visible orbit lines are drawn separately (.tracks, flattened into the
     star's plane). The rings that carry the satellites paint nothing: a painted
     tilted plane would cut through a satellite and the browser would clip it. -->
<script lang="ts">
	// Corona streamers: angle, length and width of each.
	const streamers = [
		[8, 300, 30],
		[31, 230, 18],
		[52, 260, 22],
		[76, 215, 16],
		[97, 320, 34],
		[121, 225, 18],
		[138, 250, 20],
		[156, 210, 16],
		[171, 290, 28],
		[193, 220, 18],
		[214, 265, 24],
		[233, 215, 16],
		[249, 315, 32],
		[271, 225, 18],
		[291, 250, 20],
		[311, 215, 16],
		[330, 285, 26],
		[349, 225, 18]
	];
</script>

<div class="scene" aria-hidden="true">
	<!-- Shared paint and filters for the satellites and orbit lines. -->
	<svg class="defs" width="0" height="0" focusable="false">
		<defs>
			<!-- Crinkled foil: fine noise used as a height map and lit from the upper left. -->
			<filter id="ol-foil" x="-20%" y="-20%" width="140%" height="140%" color-interpolation-filters="sRGB">
				<feTurbulence type="fractalNoise" baseFrequency="0.22 0.3" numOctaves="2" seed="11" result="noise" />
				<feSpecularLighting in="noise" surfaceScale="2" specularConstant="0.9" specularExponent="20" lighting-color="#fff7e0" result="shine">
					<fePointLight x="20" y="-30" z="60" />
				</feSpecularLighting>
				<feDiffuseLighting in="noise" surfaceScale="1.1" diffuseConstant="1.25" lighting-color="#ffffff" result="relief">
					<feDistantLight azimuth="225" elevation="62" />
				</feDiffuseLighting>
				<feBlend in="SourceGraphic" in2="relief" mode="multiply" result="shaded" />
				<feComposite in="shine" in2="shaded" operator="arithmetic" k1="0" k2="0.75" k3="1" k4="0" result="lit" />
				<feComposite in="lit" in2="SourceAlpha" operator="in" />
			</filter>
			<!-- Solar cells: a glassy glint that follows a faint unevenness in the panel. -->
			<filter id="ol-glass" x="-5%" y="-10%" width="110%" height="120%" color-interpolation-filters="sRGB">
				<feTurbulence type="fractalNoise" baseFrequency="0.06 0.5" numOctaves="2" seed="5" result="noise" />
				<feSpecularLighting in="noise" surfaceScale="1.4" specularConstant="0.9" specularExponent="26" lighting-color="#cfe6ff" result="glint">
					<feDistantLight azimuth="240" elevation="58" />
				</feSpecularLighting>
				<feComposite in="glint" in2="SourceGraphic" operator="arithmetic" k1="0" k2="0.55" k3="1" k4="0" result="lit" />
				<feComposite in="lit" in2="SourceAlpha" operator="in" />
			</filter>
			<filter id="ol-soft" x="-60%" y="-60%" width="220%" height="220%">
				<feGaussianBlur stdDeviation="5" />
			</filter>
			<filter id="ol-line-glow" x="-10%" y="-10%" width="120%" height="120%">
				<feGaussianBlur stdDeviation="0.5" />
			</filter>

			<linearGradient id="ol-cells" x1="0" y1="0" x2="1" y2="1">
				<stop offset="0" stop-color="#5d86ff" />
				<stop offset="0.45" stop-color="#2440c4" />
				<stop offset="1" stop-color="#101c6e" />
			</linearGradient>
			<linearGradient id="ol-cells-violet" x1="0" y1="0" x2="1" y2="1">
				<stop offset="0" stop-color="#a98bff" />
				<stop offset="0.45" stop-color="#5a3fd0" />
				<stop offset="1" stop-color="#25176e" />
			</linearGradient>
			<pattern id="ol-grid" width="6.5" height="8" patternUnits="userSpaceOnUse">
				<path d="M6.5 0V8M0 8H6.5" stroke="#060a22" stroke-width="0.9" fill="none" />
			</pattern>
			<linearGradient id="ol-gold" x1="0" y1="0" x2="1" y2="1">
				<stop offset="0" stop-color="#ffe9a8" />
				<stop offset="0.5" stop-color="#e2a53c" />
				<stop offset="1" stop-color="#8a5412" />
			</linearGradient>
			<linearGradient id="ol-silver" x1="0" y1="0" x2="1" y2="1">
				<stop offset="0" stop-color="#ffffff" />
				<stop offset="0.5" stop-color="#b9bfd8" />
				<stop offset="1" stop-color="#5b6185" />
			</linearGradient>
			<radialGradient id="ol-dish" cx="0.4" cy="0.3" r="0.8">
				<stop offset="0" stop-color="#ffffff" />
				<stop offset="1" stop-color="#8d94b8" />
			</radialGradient>
			<radialGradient id="ol-halo-cyan">
				<stop offset="0" stop-color="#9fe7ff" stop-opacity="0.75" />
				<stop offset="1" stop-color="#9fe7ff" stop-opacity="0" />
			</radialGradient>
			<radialGradient id="ol-halo-violet">
				<stop offset="0" stop-color="#b69cff" stop-opacity="0.7" />
				<stop offset="1" stop-color="#b69cff" stop-opacity="0" />
			</radialGradient>

			<!-- Orbit lines: bright on the near side, fading toward the far side. -->
			<linearGradient id="ol-track" x1="0" y1="0" x2="0" y2="1">
				<stop offset="0" stop-color="#7aa2ff" stop-opacity="0.1" />
				<stop offset="1" stop-color="#9fc0ff" stop-opacity="0.75" />
			</linearGradient>
			<linearGradient id="ol-track-violet" x1="0" y1="0" x2="0" y2="1">
				<stop offset="0" stop-color="#b69cff" stop-opacity="0.08" />
				<stop offset="1" stop-color="#cbb8ff" stop-opacity="0.6" />
			</linearGradient>
		</defs>
	</svg>

	<div class="sun">
		<div class="glow"></div>

		<!-- Corona: soft streamers, pulled into wisps by a displacement map. -->
		<div class="layer corona">
			<svg viewBox="0 0 760 760" focusable="false">
				<defs>
					<filter id="ol-wisps" x="-10%" y="-10%" width="120%" height="120%" color-interpolation-filters="sRGB">
						<feTurbulence type="fractalNoise" baseFrequency="0.011" numOctaves="3" seed="3" result="noise" />
						<feDisplacementMap in="SourceGraphic" in2="noise" scale="60" xChannelSelector="R" yChannelSelector="G" result="bent" />
						<feGaussianBlur in="bent" stdDeviation="10" />
					</filter>
					<radialGradient id="ol-corona">
						<stop offset="0.3" stop-color="#ffd27a" stop-opacity="0.85" />
						<stop offset="0.55" stop-color="#ff9a3d" stop-opacity="0.32" />
						<stop offset="1" stop-color="#ff7a2d" stop-opacity="0" />
					</radialGradient>
					<linearGradient id="ol-streamer" x1="0" y1="0" x2="1" y2="0">
						<stop offset="0" stop-color="#ffe2a0" stop-opacity="0.5" />
						<stop offset="1" stop-color="#ff8a3d" stop-opacity="0" />
					</linearGradient>
				</defs>
				<g filter="url(#ol-wisps)">
					<circle cx="380" cy="380" r="230" fill="url(#ol-corona)" />
					{#each streamers as [angle, length, width]}
						<path
							d="M110 {-width / 2} Q{length * 0.6} {-width / 5} {length} 0 Q{length * 0.6} {width / 5} 110 {width / 2}Z"
							fill="url(#ol-streamer)"
							transform="translate(380 380) rotate({angle})"
						/>
					{/each}
				</g>
			</svg>
		</div>

		<!-- Surface: boiling granules. Noise becomes a height map, light gives it relief. -->
		<div class="layer surface">
			<svg viewBox="0 0 760 760" focusable="false">
				<defs>
					<filter id="ol-granules" x="0" y="0" width="100%" height="100%" color-interpolation-filters="sRGB">
						<feTurbulence type="fractalNoise" baseFrequency="0.075" numOctaves="4" seed="8" result="fine" />
						<feDiffuseLighting in="fine" surfaceScale="3.4" diffuseConstant="1.3" lighting-color="#ffe2a6" result="relief">
							<feDistantLight azimuth="225" elevation="64" />
						</feDiffuseLighting>
						<!-- Large, slow cells: brighter and darker regions. -->
						<feTurbulence type="turbulence" baseFrequency="0.013" numOctaves="2" seed="21" result="coarse" />
						<feColorMatrix
							in="coarse"
							type="matrix"
							values="0 0 0 0 1
							        0 0 0 0 0.93
							        0 0 0 0 0.7
							        1.9 0 0 0 -0.3"
							result="hot"
						/>
						<feBlend in="SourceGraphic" in2="relief" mode="multiply" result="textured" />
						<feBlend in="hot" in2="textured" mode="screen" result="surface" />
						<feComposite in="surface" in2="SourceAlpha" operator="in" />
					</filter>
					<radialGradient id="ol-photosphere">
						<stop offset="0" stop-color="#fff3cf" />
						<stop offset="0.55" stop-color="#ffc45c" />
						<stop offset="1" stop-color="#ff8f2e" />
					</radialGradient>
				</defs>
				<circle cx="380" cy="380" r="120" fill="url(#ol-photosphere)" filter="url(#ol-granules)" />
			</svg>
		</div>

		<!-- Shading that stays put while the surface turns: darker toward the
		     edge, a hot spot facing the viewer, and a thin glowing rim. -->
		<div class="layer">
			<svg viewBox="0 0 760 760" focusable="false">
				<defs>
					<radialGradient id="ol-limb">
						<stop offset="0.45" stop-color="#b8330a" stop-opacity="0" />
						<stop offset="0.82" stop-color="#b8330a" stop-opacity="0.38" />
						<stop offset="1" stop-color="#7a1a02" stop-opacity="0.86" />
					</radialGradient>
					<radialGradient id="ol-hot" cx="0.4" cy="0.36" r="0.55">
						<stop offset="0" stop-color="#ffffff" stop-opacity="0.62" />
						<stop offset="0.5" stop-color="#fff0c0" stop-opacity="0.16" />
						<stop offset="1" stop-color="#fff0c0" stop-opacity="0" />
					</radialGradient>
					<filter id="ol-rim" x="-20%" y="-20%" width="140%" height="140%">
						<feGaussianBlur stdDeviation="3.2" />
					</filter>
				</defs>
				<circle cx="380" cy="380" r="120" fill="url(#ol-limb)" />
				<circle cx="380" cy="380" r="120" fill="url(#ol-hot)" />
				<circle cx="380" cy="380" r="121" fill="none" stroke="#ffb04a" stroke-width="5" filter="url(#ol-rim)" />
			</svg>
		</div>

		<!-- Lens flare: a long streak and a short cross, as a camera would see it. -->
		<div class="layer flare">
			<svg viewBox="0 0 760 760" focusable="false">
				<defs>
					<filter id="ol-streak" filterUnits="userSpaceOnUse" x="0" y="330" width="760" height="100">
						<feGaussianBlur stdDeviation="22 2.6" />
					</filter>
					<filter id="ol-spike" filterUnits="userSpaceOnUse" x="330" y="130" width="100" height="500">
						<feGaussianBlur stdDeviation="2.2 14" />
					</filter>
				</defs>
				<ellipse cx="380" cy="380" rx="300" ry="3" fill="#ffe7b8" opacity="0.32" filter="url(#ol-streak)" />
				<ellipse cx="380" cy="380" rx="2.6" ry="170" fill="#ffe7b8" opacity="0.2" filter="url(#ol-spike)" />
			</svg>
		</div>
	</div>

	<div class="tracks">
		<div class="ring r1">
			<svg viewBox="0 0 100 100" focusable="false">
				<circle cx="50" cy="50" r="49.5" fill="none" stroke="url(#ol-track)" stroke-width="1" filter="url(#ol-line-glow)" opacity="0.4" />
				<circle cx="50" cy="50" r="49.5" fill="none" stroke="url(#ol-track)" stroke-width="0.32" />
			</svg>
		</div>
		<div class="ring r2">
			<svg viewBox="0 0 100 100" focusable="false">
				<circle cx="50" cy="50" r="49.5" fill="none" stroke="url(#ol-track-violet)" stroke-width="0.4" stroke-dasharray="1.1 1.3" />
			</svg>
		</div>
		<div class="ring r3">
			<svg viewBox="0 0 100 100" focusable="false">
				<circle cx="50" cy="50" r="49.5" fill="none" stroke="url(#ol-track)" stroke-width="0.3" opacity="0.5" />
			</svg>
		</div>
	</div>

	{#snippet satellite(body: string, cells: string, halo: string, lamp: string)}
		<svg viewBox="0 0 132 72" focusable="false">
			<circle cx="66" cy="38" r="26" fill="url(#{halo})" filter="url(#ol-soft)" />
			<!-- booms -->
			<path d="M44 38H54M78 38H88" stroke="#c9cfe8" stroke-width="2.2" />
			<!-- panels -->
			<g filter="url(#ol-glass)">
				<rect x="3" y="24" width="41" height="28" rx="1.5" fill="url(#{cells})" />
				<rect x="88" y="24" width="41" height="28" rx="1.5" fill="url(#{cells})" />
			</g>
			<rect x="3" y="24" width="41" height="28" rx="1.5" fill="url(#ol-grid)" />
			<rect x="88" y="24" width="41" height="28" rx="1.5" fill="url(#ol-grid)" />
			<path d="M3.6 24.6h39.8v26.8H3.6zM88.6 24.6h39.8v26.8H88.6z" fill="none" stroke="#dfe6ff" stroke-opacity="0.55" stroke-width="0.9" />
			<!-- body wrapped in foil -->
			<rect x="54" y="22" width="24" height="32" rx="3.5" fill="url(#{body})" filter="url(#ol-foil)" />
			<rect x="54.5" y="22.5" width="23" height="31" rx="3" fill="none" stroke="#fff" stroke-opacity="0.35" />
			<rect x="59" y="42" width="14" height="6" rx="1" fill="#0b1030" fill-opacity="0.75" />
			<!-- dish and antenna -->
			<path d="M66 22V15" stroke="#c9cfe8" stroke-width="1.8" />
			<path d="M55 9a11 7 0 0 0 22 0z" fill="url(#ol-dish)" />
			<path d="M66 9V3" stroke="#e6eaff" stroke-width="1" />
			<circle cx="66" cy="2.6" r="1.8" fill="#fff" />
			<path d="M72 54l5 9" stroke="#c9cfe8" stroke-width="1" />
			<circle cx="77.4" cy="63.6" r="1.7" fill={lamp} />
		</svg>
	{/snippet}

	<div class="ring r1">
		<div class="arm">
			<div class="pivot">
				<div class="sat">{@render satellite('ol-gold', 'ol-cells', 'ol-halo-cyan', '#7ef0b0')}</div>
			</div>
		</div>
	</div>
	<div class="ring r2">
		<div class="arm">
			<div class="pivot">
				<div class="sat small">{@render satellite('ol-silver', 'ol-cells-violet', 'ol-halo-violet', '#ff7a90')}</div>
			</div>
		</div>
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
	.defs {
		position: absolute;
	}
	svg:not(.defs) {
		display: block;
		width: 100%;
		height: 100%;
		overflow: visible;
	}

	/* The star: a stack of layers, 76% of the scene so the corona has room. */
	.sun {
		position: absolute;
		inset: 12%;
	}
	.layer {
		position: absolute;
		inset: 0;
	}
	.glow {
		position: absolute;
		inset: 8%;
		border-radius: 50%;
		background: radial-gradient(circle, rgba(255, 200, 87, 0.5), rgba(255, 138, 61, 0.12) 45%, transparent 70%);
		animation: pulse 6s ease-in-out infinite;
	}
	.corona {
		will-change: transform;
		animation: turn 160s linear infinite;
	}
	.surface {
		will-change: transform;
		animation: turn 110s linear infinite reverse;
	}
	.flare {
		animation: flicker 7s ease-in-out infinite;
	}

	.ring {
		--pitch: 68deg;
		position: absolute;
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
		/* start part-way round so the two don't begin stacked behind the star */
		--delay: -9s;
	}
	.r3 {
		width: 112%;
		height: 112%;
		--tilt: 5deg;
	}

	.arm {
		position: absolute;
		inset: 0;
		transform-style: preserve-3d;
		animation: spin var(--period) linear infinite var(--dir, normal);
		animation-delay: var(--delay, 0s);
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
		animation-delay: var(--delay, 0s);
	}

	.sat {
		--w: 66px;
		position: absolute;
		width: var(--w);
		height: calc(var(--w) * 72 / 132);
		margin: calc(var(--w) * -0.29) calc(var(--w) / -2);
		transform: rotateX(calc(-1 * var(--pitch))) rotate(-16deg);
	}
	.sat.small {
		--w: 46px;
		transform: rotateX(calc(-1 * var(--pitch))) rotate(12deg);
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

	@media (max-width: 600px) {
		.sat {
			--w: 52px;
		}
		.sat.small {
			--w: 38px;
		}
	}

	@keyframes turn {
		to {
			transform: rotate(1turn);
		}
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
	@keyframes flicker {
		50% {
			opacity: 0.55;
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
