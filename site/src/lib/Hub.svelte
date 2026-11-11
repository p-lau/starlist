<!-- The "scale out" diagram, in the hero's line-art style: three apps (stars),
     each with its own satellite, beaming reports down to the ground station
     (the hub), which answers the agent.

     Everything is drawn once. All motion is CSS transforms and opacity on
     small groups, so the browser moves layers instead of redrawing.

     A satellite goes round its star on a tilted ellipse without being
     squashed: the ellipse is a circle scaled flat, the satellite is carried
     round by one rotation, and its own group turns back by the same amount
     and is stretched back by the same scale, so it stays upright. -->
<script lang="ts">
	const W = 560;
	const H = 420;
	const FLAT = 0.34; // how flat the orbits look

	type App = { name: string; x: number; y: number; r: number; orbit: number; tilt: number; period: number; delay: number };
	const apps: App[] = [
		{ name: 'api', x: 96, y: 92, r: 24, orbit: 50, tilt: -14, period: 9, delay: 0 },
		{ name: 'checkout', x: 280, y: 64, r: 30, orbit: 60, tilt: 10, period: 12, delay: -4 },
		{ name: 'pricing', x: 462, y: 104, r: 21, orbit: 44, tilt: -6, period: 8, delay: -2.5 }
	];

	// The ground station: a dish on the curved edge of the ground.
	const station = { x: 236, y: 318 };
	const dish = { x: station.x, y: station.y - 44 };
	const agent = { x: 470, y: 300 };

	// A star as a little globe: outline, equator, a tilted parallel and two meridians.
	function globe(r: number) {
		const e = (rx: number, ry: number, cy = 0) => `M${-rx} ${cy}a${rx} ${ry} 0 1 0 ${2 * rx} 0a${rx} ${ry} 0 1 0 ${-2 * rx} 0`;
		return [e(r, r * 0.3), e(r * 0.82, r * 0.22, -r * 0.55), e(r * 0.55, r)].join('') + `M0 ${-r}V${r}`;
	}

	// The ground: the top of a wide circle, with a few lines of latitude.
	const groundR = 760;
	const groundCy = station.y + groundR - 6;
	const ground = (rr: number) => {
		const y = groundCy - Math.sqrt(rr * rr - 330 * 330);
		return `M${station.x - 330} ${y}A${rr} ${rr} 0 0 1 ${station.x + 330} ${y}`;
	};
</script>

<svg viewBox="0 0 {W} {H}" focusable="false" aria-hidden="true" class="hub">
	<defs>
		<radialGradient id="hb-star">
			<stop offset="0" stop-color="#ffc857" stop-opacity="0.45" />
			<stop offset="0.45" stop-color="#ff8a3d" stop-opacity="0.16" />
			<stop offset="1" stop-color="#ff8a3d" stop-opacity="0" />
		</radialGradient>
		<radialGradient id="hb-core">
			<stop offset="0.6" stop-color="#0b0912" />
			<stop offset="1" stop-color="#2a1608" />
		</radialGradient>
		<radialGradient id="hb-ground" cx={station.x} cy={groundCy} r={groundR} gradientUnits="userSpaceOnUse">
			<stop offset="0.93" stop-color="#090a18" />
			<stop offset="1" stop-color="#1b1640" />
		</radialGradient>
		<radialGradient id="hb-dishglow">
			<stop offset="0" stop-color="#b69cff" stop-opacity="0.5" />
			<stop offset="1" stop-color="#b69cff" stop-opacity="0" />
		</radialGradient>
	</defs>

	<!-- Ground, with its edge lit violet -->
	<circle class="ground" cx={station.x} cy={groundCy} r={groundR} />
	<path class="lat" d={ground(groundR - 22)} />
	<path class="lat faint" d={ground(groundR - 52)} />
	{#each [-260, -170, -80, 80, 170, 260] as dx}
		<path class="lat faint" d="M{station.x + dx} {groundCy - Math.sqrt(groundR ** 2 - dx ** 2)}Q{station.x + dx * 1.18} {station.y + 70} {station.x + dx * 1.5} {H + 20}" />
	{/each}

	<!-- Beams from each satellite's star down to the dish, with reports travelling down them -->
	{#each apps as a, i}
		<path class="beam" d="M{a.x} {a.y + a.r + 6}L{dish.x} {dish.y - 6}" />
		{#each [0, 1] as k}
			<g transform="translate({a.x} {a.y + a.r + 6})">
				<g
					class="packet"
					style="--dx:{dish.x - a.x}px; --dy:{dish.y - 6 - a.y - a.r - 6}px; animation-delay:{-(i * 0.9 + k * 1.6)}s"
				>
					<rect x="-3" y="-3" width="6" height="6" rx="1.2" transform="rotate(45)" />
				</g>
			</g>
		{/each}
	{/each}

	<!-- The hub answers the agent -->
	<path class="beam answer" d="M{dish.x + 16} {dish.y + 10}Q{(dish.x + agent.x) / 2} {dish.y - 30} {agent.x - 34} {agent.y - 6}" />
	<g transform="translate({agent.x - 34} {agent.y - 6})"><circle class="ping" r="4" /></g>

	<!-- Apps: a glow, a globe, its orbit and a satellite going round it -->
	{#each apps as a}
		<g transform="translate({a.x} {a.y})" style="--period:{a.period}s; --delay:{a.delay}s">
			<circle r={a.r * 2.6} fill="url(#hb-star)" />
			<!-- far half of the orbit, behind the star -->
			<g transform="rotate({a.tilt}) scale(1 {FLAT})">
				<path class="orbit" d="M{-a.orbit} 0A{a.orbit} {a.orbit} 0 0 1 {a.orbit} 0" />
			</g>
			<circle r={a.r} fill="url(#hb-core)" />
			<path class="globe" d={globe(a.r)} />
			<circle class="rim" r={a.r} />
			<!-- near half, in front -->
			<g transform="rotate({a.tilt}) scale(1 {FLAT})">
				<path class="orbit near" d="M{a.orbit} 0A{a.orbit} {a.orbit} 0 0 1 {-a.orbit} 0" />
			</g>
			<g transform="rotate({a.tilt}) scale(1 {FLAT})">
				<g class="carry">
					<g transform="translate({a.orbit} 0)">
						<g class="uncarry">
							<g transform="scale(1 {1 / FLAT}) rotate({-a.tilt})">
								<g class="sat">
									<path d="M-13 -3h8v6h-8zM5 -3h8v6h-8z" class="panel" />
									<path d="M-5 0H-3M3 0H5" />
									<rect x="-3" y="-4" width="6" height="8" rx="1" class="body" />
									<path d="M0 -4V-8" />
									<circle cy="-9" r="1.4" class="lamp" />
								</g>
							</g>
						</g>
					</g>
				</g>
			</g>
			<text y={Math.max(a.r, a.orbit * FLAT) + 22} class="label">{a.name}</text>
		</g>
	{/each}

	<!-- Ground station: a dish on a mast, a small building, and a pulse when a report lands -->
	<g transform="translate({dish.x} {dish.y})">
		<circle r="40" fill="url(#hb-dishglow)" />
		<circle class="pulse" r="14" />
		<circle class="pulse late" r="14" />
	</g>
	<g class="station">
		<path class="mast" d="M{dish.x} {dish.y + 8}L{station.x - 12} {station.y - 2}M{dish.x} {dish.y + 8}L{station.x + 12} {station.y - 2}M{dish.x} {dish.y + 8}V{station.y - 2}" />
		<path class="building" d="M{station.x - 30} {station.y + 4}V{station.y - 10}H{station.x + 30}V{station.y + 4}M{station.x - 18} {station.y - 3}h8M{station.x + 10} {station.y - 3}h8" />
		<g transform="translate({dish.x} {dish.y}) rotate(-12)">
			<path class="dish" d="M-26 0A26 9 0 0 0 26 0A26 9 0 0 0 -26 0Z" />
			<path class="dish-inner" d="M-17 1A17 5.5 0 0 0 17 1" />
			<path d="M-20 3L0 -24M20 3L0 -24M0 6V-24" class="feed" />
			<circle cy="-25" r="2.2" class="lamp hubl" />
		</g>
		<text x={station.x} y={station.y + 30} class="label hub-label">ground station</text>
	</g>

	<!-- The agent -->
	<g transform="translate({agent.x} {agent.y})">
		<rect x="-34" y="-22" width="68" height="40" rx="6" class="term" />
		<path d="M-34 -12H34" class="term-line" />
		<circle cx="-27" cy="-17" r="1.3" class="dot" /><circle cx="-22" cy="-17" r="1.3" class="dot" />
		<path d="M-25 -2l5 4-5 4M-14 7h12" class="prompt" />
		<text y="36" class="label">agent</text>
	</g>
</svg>

<style>
	.hub {
		width: 100%;
		height: auto;
		overflow: visible;
		/* the ground runs off the bottom: fade it out */
		mask-image: linear-gradient(#000 78%, transparent 98%);
	}
	path,
	circle,
	rect {
		fill: none;
		stroke-linecap: round;
		stroke-linejoin: round;
	}

	.ground {
		fill: url(#hb-ground);
		stroke: var(--violet);
		stroke-width: 1.4;
		stroke-opacity: 0.8;
	}
	.lat {
		stroke: var(--violet);
		stroke-width: 0.8;
		opacity: 0.35;
	}
	.lat.faint {
		opacity: 0.16;
	}

	.globe {
		stroke: #ffc857;
		stroke-width: 0.8;
		opacity: 0.35;
	}
	.rim {
		stroke: #ffd98a;
		stroke-width: 1.4;
	}
	.orbit {
		stroke: var(--orbit);
		stroke-width: 2.2;
		stroke-dasharray: 2 9;
		opacity: 0.35;
		vector-effect: non-scaling-stroke;
	}
	.orbit.near {
		opacity: 0.75;
	}
	.label {
		stroke: none;
		font: 500 11px var(--font-mono);
		letter-spacing: 0.08em;
		fill: var(--muted);
		text-anchor: middle;
	}
	.hub-label {
		fill: var(--violet);
	}

	/* the satellite */
	.carry {
		animation: carry var(--period) linear infinite;
		animation-delay: var(--delay);
	}
	.uncarry {
		animation: carry var(--period) linear infinite reverse;
		animation-delay: var(--delay);
	}
	.sat {
		stroke: var(--sat);
		stroke-width: 1;
		animation: behind var(--period) linear infinite;
		animation-delay: var(--delay);
	}
	.sat .panel {
		fill: rgba(79, 108, 255, 0.35);
	}
	.sat .body {
		fill: #0b0f2a;
	}
	.lamp {
		fill: #7ef0b0;
		stroke: none;
	}
	@keyframes carry {
		to {
			transform: rotate(1turn);
		}
	}
	/* Hidden while it passes behind the star: the far side is 180 to 360 degrees
	   round (angles grow clockwise from the right end), and every orbit is about
	   twice its star's radius, so the star covers roughly 243 to 297 degrees. */
	@keyframes behind {
		0%,
		65%,
		85%,
		100% {
			opacity: 1;
		}
		68.5%,
		81.5% {
			opacity: 0;
		}
	}

	/* reports travelling down to the dish */
	.beam {
		stroke: var(--sat);
		stroke-width: 1;
		stroke-dasharray: 1 6;
		opacity: 0.45;
	}
	.beam.answer {
		stroke: var(--violet);
	}
	.packet {
		fill: var(--sat);
		stroke: none;
		animation: travel 3.2s cubic-bezier(0.5, 0, 0.7, 1) infinite;
	}
	.packet rect {
		fill: var(--sat);
	}
	@keyframes travel {
		0% {
			transform: translate(0, 0);
			opacity: 0;
		}
		12% {
			opacity: 1;
		}
		88% {
			opacity: 1;
		}
		100% {
			transform: translate(var(--dx), var(--dy));
			opacity: 0;
		}
	}

	/* the station */
	.station {
		stroke: var(--violet);
		stroke-width: 1.4;
	}
	.mast {
		stroke-width: 1.2;
	}
	.building {
		fill: #0d0f24;
	}
	.dish {
		fill: #12143a;
		stroke: #d3c4ff;
	}
	.dish-inner {
		stroke-opacity: 0.5;
	}
	.feed {
		stroke-width: 1;
	}
	.lamp.hubl {
		fill: #ff7a90;
		animation: blink 2s steps(1) infinite;
	}
	.pulse {
		stroke: var(--violet);
		stroke-width: 1.2;
		transform-box: fill-box;
		transform-origin: center;
		animation: pulse 2.4s ease-out infinite;
	}
	.pulse.late {
		animation-delay: -1.2s;
	}
	@keyframes pulse {
		from {
			transform: scale(0.6);
			opacity: 0.8;
		}
		to {
			transform: scale(3.4);
			opacity: 0;
		}
	}
	.ping {
		fill: var(--violet);
		transform-box: fill-box;
		transform-origin: center;
		animation: ping 2.4s ease-out infinite;
	}
	@keyframes ping {
		0%,
		60% {
			transform: scale(0.4);
			opacity: 0;
		}
		75% {
			transform: scale(1.4);
			opacity: 1;
		}
		100% {
			transform: scale(2.2);
			opacity: 0;
		}
	}
	@keyframes blink {
		0%,
		20% {
			opacity: 1;
		}
		10%,
		30%,
		100% {
			opacity: 0.15;
		}
	}

	/* the agent's terminal */
	.term {
		fill: #0b0d22;
		stroke: var(--line-strong);
		stroke-width: 1.2;
	}
	.term-line {
		stroke: var(--line-strong);
	}
	.dot {
		fill: var(--faint);
		stroke: none;
	}
	.prompt {
		stroke: var(--star);
		stroke-width: 1.6;
	}

	@media (prefers-reduced-motion: reduce) {
		.carry,
		.uncarry,
		.sat,
		.packet,
		.pulse,
		.ping,
		.lamp {
			animation: none;
		}
	}
</style>
