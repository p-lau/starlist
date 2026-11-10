<script lang="ts">
	import Starfield from '#lib/Starfield.svelte';
	import Orbit from '#lib/Orbit.svelte';
	import Hub from '#lib/Hub.svelte';
	import Terminal from '#lib/Terminal.svelte';
	import CopyCode from '#lib/CopyCode.svelte';
	import { reveal } from '#lib/reveal.ts';
	import { spotlight, tilt } from '#lib/effects.ts';

	const repo = 'https://github.com/p-lau/starlist';
	const title = 'Starlist: your logs, distilled for AI agents';
	const description =
		'A tiny open-source MCP server that orbits your app, watches its logs, groups them into patterns, redacts secrets and tells agents what changed.';

	const stats = [
		{ v: '~5 MB', l: 'static binary' },
		{ v: '~11 MB', l: 'memory while serving' },
		{ v: '0', l: 'external services' },
		{ v: 'Any', l: 'language that writes logs' }
	];

	const tools = [
		{
			name: 'diagnose',
			body: 'Start here. The likely causes of what is wrong now, ranked in one call, each with the deploy it started with, its code location and sample lines.'
		},
		{
			name: 'whats_new',
			body: 'What appeared for the first time, came back, spiked, is slowly rising or went quiet versus a baseline. Spikes are judged against overall traffic, so a busy hour is not an incident.'
		},
		{
			name: 'verify_fix',
			body: 'Did the error stop after your fix? Clear verdicts like fixed, reduced, still_occurring or too_early, with how long to wait.'
		},
		{
			name: 'compare_deploy',
			body: 'Everything that changed around a deploy: new patterns that are likely regressions, what increased, what stopped.'
		},
		{
			name: 'list_patterns · get_pattern',
			body: 'Search patterns by text or level. Counts, an hourly histogram and redacted samples with their stack traces.'
		},
		{
			name: 'get_context · get_request',
			body: 'The redacted lines around an error, or every event of one request in order, so the agent sees what led up to it without the log file.'
		},
		{
			name: 'set_pattern_state · get_release',
			body: 'Resolve a pattern after a verified fix and hear about it if it regresses. Each release gets a report card: new, regressed and resolved.'
		},
		{
			name: 'add_marker · list_markers',
			body: 'Name a moment like "fix-currency". Deploys are also picked up from lines like "Starting api version 1.4.2".'
		},
		{
			name: 'Output budget',
			body: 'Brief by default, full on request, paged when large. Answers stay small enough to fit in an agent’s context.'
		}
	];

	const steps = [
		{
			n: '01',
			t: 'Collect',
			b: 'Follows files like tail -F, stdin, Docker containers, journald units, Kubernetes pods, OpenTelemetry logs or a Loki stream. Stack trace lines stay attached to the log line that produced them.'
		},
		{
			n: '02',
			t: 'Redact',
			b: 'Emails, IPs, bearer tokens, JWTs, cloud and API keys, passwords, card numbers and private keys are scrubbed before anything is stored. Add your own patterns too.'
		},
		{
			n: '03',
			t: 'Pattern',
			b: 'Numbers, IDs, UUIDs, hex and timestamps are masked. The pattern ID is a hash of the template, so it is the same across restarts and hosts.'
		},
		{
			n: '04',
			t: 'Store',
			b: 'A local SQLite file with per-10-second counts, recent samples and markers. After a week counts fold into hourly ones, kept for 400 days.'
		}
	];

	const rawLines = [
		'09:41:02.113 INFO  GET /api/cart 200 12ms user=8812',
		'09:41:02.140 INFO  GET /api/cart 200 9ms user=1204',
		'09:41:02.201 WARN  retrying payment provider, attempt 2',
		'09:41:02.377 ERROR charge failed for order 99812: "EURX"',
		'Traceback (most recent call last):',
		'  File "checkout/pay.py", line 88, in charge',
		'KeyError: \'EURX\'',
		'09:41:02.390 INFO  POST /api/pay 500 41ms user=3321',
		'09:41:02.411 DEBUG cache hit key=sess:ab31f0…',
		'09:41:02.452 INFO  token=eyJhbGciOiJI… refreshed',
		'09:41:02.480 WARN  retrying payment provider, attempt 3',
		'09:41:02.502 ERROR charge failed for order 99813: "EURX"',
		'09:41:02.519 INFO  GET /api/cart 200 11ms user=7740'
	];
</script>

<svelte:head>
	<title>{title}</title>
	<meta name="description" content={description} />
	<meta property="og:title" content={title} />
	<meta property="og:description" content={description} />
	<meta property="og:type" content="website" />
	<meta name="twitter:card" content="summary" />
</svelte:head>

<Starfield />

<div class="page" {@attach spotlight}>
	<div class="aurora" aria-hidden="true"><i></i><i></i><i></i></div>
	<header class="nav wrap">
		<a class="brand" href="#top" aria-label="Starlist home">
			<svg viewBox="0 0 32 32" width="26" height="26" aria-hidden="true">
				<circle cx="16" cy="16" r="6.5" fill="url(#bg)" />
				<ellipse cx="16" cy="16" rx="13" ry="5" fill="none" stroke="#7aa2ff" stroke-width="1.4" transform="rotate(-25 16 16)" />
				<circle cx="27.2" cy="10.9" r="2.2" fill="#9fe7ff" />
				<defs>
					<radialGradient id="bg"><stop offset="0" stop-color="#fff6d8" /><stop offset=".6" stop-color="#ffc857" /><stop offset="1" stop-color="#ff8a3d" /></radialGradient>
				</defs>
			</svg>
			Starlist
		</a>
		<nav aria-label="Sections">
			<a href="#demo">Demo</a>
			<a href="#tools">Tools</a>
			<a href="#how">How it works</a>
			<a href="#install">Install</a>
		</nav>
		<a class="gh" href={repo} rel="noopener">
			<svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path fill="currentColor" d="M8 0C3.58 0 0 3.58 0 8c0 3.54 2.29 6.53 5.47 7.59.4.07.55-.17.55-.38 0-.19-.01-.82-.01-1.49-2.01.37-2.53-.49-2.69-.94-.09-.23-.48-.94-.82-1.13-.28-.15-.68-.52-.01-.53.63-.01 1.08.58 1.23.82.72 1.21 1.87.87 2.33.66.07-.52.28-.87.51-1.07-1.78-.2-3.64-.89-3.64-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82.64-.18 1.32-.27 2-.27.68 0 1.36.09 2 .27 1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48 0 1.07-.01 1.93-.01 2.2 0 .21.15.46.55.38A8.013 8.013 0 0016 8c0-4.42-3.58-8-8-8z" /></svg>
			<span>GitHub</span>
		</a>
	</header>

	<main id="top">
		<!-- HERO -->
		<section class="hero">
			<Orbit />
			<div class="hero-copy wrap">
				<p class="eyebrow">v1.0 · Open-source MCP server · Rust · MIT</p>
				<h1>
					Your logs,<br /><span class="shine">distilled for&nbsp;agents.</span>
				</h1>
				<p class="lede">
					Your apps are stars. Starlist puts a small satellite in orbit around each one. It watches the
					logs, groups them into patterns, scrubs out secrets and tells your AI agent what actually
					changed, in a few hundred tokens instead of a firehose.
				</p>
				<div class="ctas">
					<a class="btn btn-primary" href="#install">Get started</a>
					<a class="btn btn-ghost" href={repo} rel="noopener">View source</a>
				</div>
			</div>
		</section>

		<section class="stats wrap" aria-label="At a glance">
			{#each stats as s}
				<div class="stat">
					<strong>{s.v}</strong>
					<span>{s.l}</span>
				</div>
			{/each}
		</section>

		<!-- PROBLEM -->
		<section class="problem wrap" {@attach reveal}>
			<div class="section-head">
				<p class="eyebrow">The problem</p>
				<h2>Agents drown in raw logs.</h2>
				<p>
					Hand an agent a log file and it burns its context on health checks and cache hits, and may
					read your tokens on the way. Starlist sits next to the app and does the reading for it.
				</p>
			</div>
			<div class="versus">
				<div class="pane raw" aria-label="Raw log lines">
					<div class="pane-h"><span>app.log</span><span class="muted">48,210 lines / hour</span></div>
					<div class="scroll" aria-hidden="true">
						<div class="lines">
							{#each [...rawLines, ...rawLines] as l}
								<div
									class:err={l.includes('ERROR') || l.includes('Error')}
									class:warn={l.includes('WARN')}
									class:secret={l.includes('token=')}
								>
									{l}
								</div>
							{/each}
						</div>
					</div>
				</div>
				<div class="arrow" aria-hidden="true">
					<svg viewBox="0 0 80 24" width="80" height="24"><path d="M0 12h70m-8-8 8 8-8 8" fill="none" stroke="url(#ag)" stroke-width="2" stroke-linecap="round" /><defs><linearGradient id="ag"><stop offset="0" stop-color="#7aa2ff" stop-opacity=".2" /><stop offset="1" stop-color="#9fe7ff" /></linearGradient></defs></svg>
				</div>
				<div class="pane clean" aria-label="What the agent receives">
					<div class="pane-h"><span>what the agent gets</span><span class="muted">3 findings</span></div>
					<ul class="findings">
						<li><b class="tag new">new</b> <code>charge failed for order &lt;*&gt;: &lt;str&gt; | KeyError</code> <em>412×</em></li>
						<li><b class="tag spike">spike</b> <code>retrying payment provider, attempt &lt;n&gt;</code> <em>17× baseline</em></li>
						<li><b class="tag quiet">quiet</b> <code>legacy tax path used for &lt;*&gt;</code> <em>0 since v1.4.2</em></li>
					</ul>
					<p class="redacted">token=<span>[REDACTED:jwt]</span> · user emails, IPs and keys never leave the box</p>
				</div>
			</div>
		</section>

		<!-- DEMO -->
		<section id="demo" class="demo wrap" {@attach reveal}>
			<div class="demo-copy">
				<p class="eyebrow">In practice</p>
				<h2>Ask in plain words. Get answers, not lines.</h2>
				<p>
					Your agent calls a handful of focused tools over MCP. Each answer is short, structured and
					already compared against a baseline, so the agent can act on it right away.
				</p>
				<ol class="flow">
					<li><span>1</span> "What's new since the deploy?"</li>
					<li><span>2</span> The agent finds the cause and ships a fix</li>
					<li><span>3</span> "Did it stop?" gets a real verdict</li>
				</ol>
				<p class="fine">Example output, trimmed for the page.</p>
			</div>
			<div class="term-frame" {@attach tilt}>
				<Terminal />
			</div>
		</section>

		<!-- TOOLS -->
		<section id="tools" class="tools wrap" {@attach reveal}>
			<div class="section-head center">
				<p class="eyebrow">The tool belt</p>
				<h2>A small set of tools that do one job well.</h2>
				<p>Times accept <code>30m</code>, <code>2h</code>, an ISO time or a marker name.</p>
			</div>
			<div class="grid stagger">
				{#each tools as tool, i}
					<article class="card spot" style="--n:{i}">
						<h3><code>{tool.name}</code></h3>
						<p>{tool.body}</p>
					</article>
				{/each}
			</div>
		</section>

		<!-- HOW -->
		<section id="how" class="how wrap" {@attach reveal}>
			<div class="section-head">
				<p class="eyebrow">How it works</p>
				<h2>One orbit, four stages.</h2>
				<p>Everything happens on the same machine as your app. No log pipeline, no hosted backend.</p>
			</div>
			<ol class="steps stagger">
				{#each steps as s, i}
					<li style="--n:{i * 2}">
						<span class="num">{s.n}</span>
						<h3>{s.t}</h3>
						<p>{s.b}</p>
					</li>
				{/each}
			</ol>
		</section>

		<!-- CONSTELLATION -->
		<section class="fleet wrap" {@attach reveal}>
			<div class="fleet-art">
				<Hub />
			</div>
			<div class="fleet-copy">
				<p class="eyebrow">Scale out</p>
				<h2>One app or a whole constellation.</h2>
				<p>
					Most teams run one satellite per app and point their agent straight at it. It speaks MCP over
					HTTP with bearer tokens, or over stdio for local tools.
				</p>
				<p>
					For larger fleets, an optional <strong>ground station</strong> hub gathers every satellite’s
					reports, so an agent can ask across services and follow one request through all of them.
					Each satellite keeps working on its own if the hub goes away.
				</p>
			</div>
		</section>

		<!-- INSTALL -->
		<section id="install" class="install wrap" {@attach reveal}>
			<div class="section-head center">
				<p class="eyebrow">Launch</p>
				<h2>In orbit in a minute.</h2>
				<p>Works with any language. Starlist only reads the text your app already writes, plain lines or JSON.</p>
			</div>
			<div class="install-grid stagger">
				<CopyCode label="Install with Cargo" code="cargo install starlist" />
				<CopyCode label="Or run the container" code="docker pull ghcr.io/p-lau/starlist:1" />
				<CopyCode label="Find your logs and set up your agent" code="starlist init" />
				<CopyCode label="Start it and leave it running" code="starlist serve" />
			</div>
			<p class="install-note">
				Prebuilt binaries for Linux, macOS and Windows are on the
				<a href="{repo}/releases/latest" rel="noopener">releases page</a>. <code>starlist init</code> sets up
				Claude Code, Cursor, VS Code or Gemini CLI; any other MCP client connects to
				<code>http://127.0.0.1:7341/mcp</code>. While it runs, a small dashboard is at
				<code>http://127.0.0.1:7341/</code>.
			</p>
		</section>
	</main>

	<footer class="wrap">
		<span>Starlist · MIT licensed</span>
		<span class="links">
			<a href="https://crates.io/crates/starlist" rel="noopener">crates.io</a>
			<a href={repo} rel="noopener">GitHub</a>
		</span>
	</footer>
</div>

<style>
	.page {
		position: relative;
		z-index: 1;
		overflow-x: clip;
	}
	.hero > *,
	.demo > *,
	.fleet > *,
	.versus > * {
		min-width: 0;
	}

	/* NAV */
	/* Slow drifting light behind the hero. Each blob is a plain gradient that
	   only ever moves by transform, so the compositor does all the work. */
	.aurora {
		position: absolute;
		top: -260px;
		left: 50%;
		width: min(1500px, 140vw);
		height: 980px;
		transform: translateX(-50%);
		z-index: -1;
		pointer-events: none;
		mask-image: linear-gradient(#000 55%, transparent);
	}
	.aurora i {
		position: absolute;
		width: 62%;
		aspect-ratio: 1;
		border-radius: 50%;
		will-change: transform;
		animation: float 22s ease-in-out infinite alternate;
	}
	.aurora i:nth-child(1) {
		left: 38%;
		top: 0;
		background: radial-gradient(closest-side, rgba(122, 162, 255, 0.34), transparent);
	}
	.aurora i:nth-child(2) {
		left: 2%;
		top: 14%;
		background: radial-gradient(closest-side, rgba(182, 156, 255, 0.26), transparent);
		animation-duration: 28s;
		animation-delay: -9s;
	}
	.aurora i:nth-child(3) {
		left: 30%;
		top: 30%;
		width: 48%;
		background: radial-gradient(closest-side, rgba(255, 138, 61, 0.2), transparent);
		animation-duration: 34s;
		animation-delay: -17s;
	}
	@keyframes float {
		from {
			transform: translate3d(-9%, -5%, 0) scale(1);
		}
		to {
			transform: translate3d(11%, 8%, 0) scale(1.18);
		}
	}

	.nav {
		display: flex;
		align-items: center;
		gap: 24px;
		position: sticky;
		top: 10px;
		z-index: 10;
		margin-top: 10px;
		padding-top: 12px;
		padding-bottom: 12px;
		border-radius: 999px;
		border: 1px solid var(--line);
		background: #090b1a;
		box-shadow: 0 10px 30px -18px rgba(0, 0, 0, 0.9);
	}
	.brand {
		display: flex;
		align-items: center;
		gap: 10px;
		font: 700 1.2rem var(--font-display);
		text-decoration: none;
		letter-spacing: -0.02em;
	}
	nav {
		display: flex;
		gap: 26px;
		margin-left: auto;
	}
	nav a {
		color: var(--muted);
		text-decoration: none;
		font-size: 0.92rem;
	}
	nav a:hover {
		color: var(--text);
	}
	.gh {
		display: inline-flex;
		align-items: center;
		gap: 8px;
		padding: 8px 14px;
		border-radius: 999px;
		border: 1px solid var(--line-strong);
		text-decoration: none;
		font-size: 0.88rem;
		color: var(--text);
		background: rgba(255, 255, 255, 0.03);
	}
	.gh:hover {
		border-color: rgba(159, 231, 255, 0.5);
	}

	/* HERO */
	/* The hero runs the full width, with the star's edge drawn behind it. The
	   space below the buttons is where the edge shows. */
	.hero {
		position: relative;
		margin-top: -96px;
		padding-top: 152px;
		padding-bottom: clamp(220px, 30vw, 380px);
	}
	.hero-copy {
		position: relative;
	}
	h1 {
		font-size: clamp(2.6rem, 6.4vw, 4.9rem);
		font-weight: 700;
		margin: 18px 0 22px;
	}
	.shine {
		background: linear-gradient(100deg, #ffe3a1 0%, var(--star) 30%, var(--star-hot) 55%, var(--violet) 100%);
		-webkit-background-clip: text;
		background-clip: text;
		color: transparent;
	}
	.lede {
		font-size: 1.15rem;
		color: var(--muted);
		max-width: 34em;
		margin: 0 0 30px;
	}
	.ctas {
		display: flex;
		gap: 12px;
		flex-wrap: wrap;
	}

	/* STATS */
	.stats {
		position: relative;
		display: grid;
		grid-template-columns: repeat(4, 1fr);
		gap: 1px;
		margin-top: -40px;
		margin-bottom: 120px;
	}
	.stat {
		padding: 22px 6px;
		border-top: 1px solid var(--line);
	}
	.stat strong {
		display: block;
		font: 700 1.9rem var(--font-display);
		color: var(--text);
	}
	.stat span {
		color: var(--faint);
		font-size: 0.9rem;
	}

	section {
		scroll-margin-top: 96px;
	}

	.section-head {
		max-width: 640px;
		margin-bottom: 44px;
	}
	.section-head.center {
		margin-left: auto;
		margin-right: auto;
		text-align: center;
	}
	h2 {
		font-size: clamp(2rem, 4.2vw, 3.1rem);
		font-weight: 650;
		margin: 14px 0 16px;
	}
	.section-head p:not(.eyebrow),
	.demo-copy > p:not(.eyebrow):not(.fine),
	.fleet-copy > p:not(.eyebrow) {
		color: var(--muted);
		font-size: 1.06rem;
		margin: 0;
	}
	.fleet-copy > p + p {
		margin-top: 14px;
	}

	/* PROBLEM */
	.problem {
		margin-bottom: 140px;
	}
	.versus {
		display: grid;
		grid-template-columns: 1fr auto 1fr;
		align-items: center;
		gap: 18px;
	}
	.pane {
		border-radius: var(--radius);
		border: 1px solid var(--line);
		background: var(--panel);
		overflow: hidden;
	}
	.pane-h {
		display: flex;
		justify-content: space-between;
		padding: 12px 16px;
		border-bottom: 1px solid var(--line);
		font: 500 0.78rem var(--font-mono);
		color: var(--text);
	}
	.muted {
		color: var(--faint);
	}
	.scroll {
		height: 260px;
		overflow: hidden;
		mask-image: linear-gradient(transparent, #000 18%, #000 82%, transparent);
	}
	.lines {
		padding: 10px 16px;
		font: 0.72rem/1.75 var(--font-mono);
		color: #7d84ad;
		white-space: nowrap;
		animation: scrollup 16s linear infinite;
	}
	.lines .err {
		color: #ff9aaa;
	}
	.lines .warn {
		color: #f5c27a;
	}
	.lines .secret {
		color: #c8b5ff;
		text-decoration: underline wavy rgba(255, 122, 144, 0.6);
	}
	@keyframes scrollup {
		to {
			transform: translateY(-50%);
		}
	}
	.clean {
		border-color: rgba(159, 231, 255, 0.28);
		box-shadow: 0 0 60px -20px rgba(159, 231, 255, 0.35);
	}
	.findings {
		list-style: none;
		margin: 0;
		padding: 14px 16px 6px;
		display: grid;
		gap: 12px;
	}
	.findings li {
		display: grid;
		grid-template-columns: auto 1fr;
		gap: 4px 10px;
		align-items: baseline;
		font-size: 0.82rem;
	}
	.findings code {
		color: var(--text);
		font-size: 0.78rem;
		word-break: break-word;
	}
	.findings em {
		grid-column: 2;
		font-style: normal;
		color: var(--faint);
		font-size: 0.78rem;
	}
	.tag {
		font: 600 0.66rem var(--font-mono);
		text-transform: uppercase;
		letter-spacing: 0.08em;
		padding: 3px 7px;
		border-radius: 6px;
	}
	.tag.new {
		color: var(--bad);
		background: rgba(255, 122, 144, 0.12);
	}
	.tag.spike {
		color: #ffb86b;
		background: rgba(255, 184, 107, 0.12);
	}
	.tag.quiet {
		color: var(--good);
		background: rgba(126, 240, 176, 0.1);
	}
	.redacted {
		margin: 10px 16px 16px;
		padding-top: 12px;
		border-top: 1px dashed var(--line);
		font: 0.74rem var(--font-mono);
		color: var(--faint);
	}
	.redacted span {
		color: var(--violet);
	}

	/* DEMO */
	.demo {
		display: grid;
		grid-template-columns: 0.8fr 1.2fr;
		gap: 56px;
		align-items: center;
		margin-bottom: 140px;
	}
	.flow {
		list-style: none;
		padding: 0;
		margin: 28px 0 0;
		display: grid;
		gap: 12px;
	}
	.flow li {
		display: flex;
		align-items: center;
		gap: 12px;
		color: var(--text);
	}
	.flow span {
		display: grid;
		place-items: center;
		width: 28px;
		height: 28px;
		flex: none;
		border-radius: 50%;
		border: 1px solid var(--line-strong);
		font: 600 0.75rem var(--font-mono);
		color: var(--sat);
	}
	.fine {
		margin: 24px 0 0;
		font-size: 0.8rem;
		color: var(--faint);
	}
	/* The terminal leans toward the pointer and has a light running round its edge:
	   a wide conic gradient spinning behind a 1px gap. */
	.term-frame {
		position: relative;
		padding: 1px;
		border-radius: calc(var(--radius) + 1px);
		overflow: hidden;
		box-shadow: 0 30px 80px -30px rgba(0, 0, 0, 0.8);
		transform: perspective(1400px) rotateX(var(--rx, 0deg)) rotateY(var(--ry, 0deg));
		transition: transform 0.25s ease-out;
	}
	.term-frame::before {
		content: '';
		position: absolute;
		left: 50%;
		top: 50%;
		width: 1400px;
		height: 1400px;
		margin: -700px;
		background: conic-gradient(
			transparent 0 62%,
			rgba(122, 162, 255, 0.9),
			var(--sat),
			var(--violet),
			transparent 96%
		);
		animation: turn 9s linear infinite;
	}
	.term-frame > :global(*) {
		position: relative;
		box-shadow: none;
		border-color: rgba(140, 160, 255, 0.16);
	}
	@keyframes turn {
		to {
			transform: rotate(1turn);
		}
	}

	/* TOOLS */
	.tools {
		margin-bottom: 140px;
	}
	.tools code {
		font-size: 0.9em;
		color: var(--sat);
	}
	.grid {
		display: grid;
		grid-template-columns: repeat(3, 1fr);
		gap: 16px;
	}
	.card {
		padding: 24px;
		border-radius: var(--radius);
		border: 1px solid var(--line);
		background: var(--panel);
		transition: translate 0.25s ease;
	}
	.card:hover {
		translate: 0 -3px;
	}
	.card h3 {
		font-size: 1rem;
		margin-bottom: 10px;
	}
	.card h3 code {
		color: var(--star);
		font-size: 0.92rem;
	}
	.card p {
		margin: 0;
		color: var(--muted);
		font-size: 0.94rem;
	}

	/* HOW */
	.how {
		margin-bottom: 140px;
	}
	.steps {
		list-style: none;
		padding: 0;
		margin: 0;
		display: grid;
		grid-template-columns: repeat(4, 1fr);
		gap: 0;
		position: relative;
	}
	.steps::before {
		content: '';
		position: absolute;
		top: 21px;
		left: 22px;
		right: 22px;
		height: 1px;
		background: linear-gradient(90deg, var(--star), var(--orbit), var(--violet), var(--sat));
		opacity: 0.5;
		transform-origin: left;
	}
	/* The line between the stages draws itself as the section scrolls in. */
	@supports (animation-timeline: view()) {
		@media (prefers-reduced-motion: no-preference) {
			.steps::before {
				animation: draw linear both;
				animation-timeline: view();
				animation-range: entry 30% cover 45%;
			}
		}
		@keyframes draw {
			from {
				transform: scaleX(0);
			}
		}
	}
	.steps li {
		position: relative;
		padding-right: 24px;
	}
	.num {
		display: grid;
		place-items: center;
		width: 44px;
		height: 44px;
		border-radius: 50%;
		background: var(--bg-2);
		border: 1px solid var(--line-strong);
		font: 600 0.8rem var(--font-mono);
		color: var(--star);
		position: relative;
		margin-bottom: 20px;
	}
	.steps h3 {
		font-size: 1.3rem;
		margin-bottom: 8px;
	}
	.steps p {
		margin: 0;
		color: var(--muted);
		font-size: 0.94rem;
	}

	/* FLEET */
	.fleet {
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 56px;
		align-items: center;
		margin-bottom: 140px;
	}
	/* INSTALL */
	.install {
		margin-bottom: 120px;
	}
	.install-grid {
		max-width: 720px;
		margin: 0 auto;
		display: grid;
		gap: 12px;
	}
	.install-note {
		max-width: 720px;
		margin: 22px auto 0;
		color: var(--muted);
		font-size: 0.94rem;
		text-align: center;
	}
	.install-note code {
		color: var(--sat);
		font-size: 0.9em;
		overflow-wrap: anywhere;
	}

	footer {
		display: flex;
		justify-content: space-between;
		padding-top: 28px;
		padding-bottom: 40px;
		border-top: 1px solid var(--line);
		color: var(--faint);
		font-size: 0.88rem;
	}
	footer a {
		color: var(--muted);
	}
	footer .links {
		display: flex;
		gap: 18px;
	}

	/* RESPONSIVE */
	@media (max-width: 960px) {
		.demo,
		.fleet {
			grid-template-columns: 1fr;
		}
		.hero {
			padding-top: 120px;
			text-align: center;
		}
		.lede {
			margin-left: auto;
			margin-right: auto;
		}
		.ctas {
			justify-content: center;
		}
		.grid {
			grid-template-columns: repeat(2, 1fr);
		}
		.steps {
			grid-template-columns: repeat(2, 1fr);
			gap: 36px 0;
		}
		.steps::before {
			display: none;
		}
		.versus {
			grid-template-columns: 1fr;
		}
		.arrow {
			justify-self: center;
			transform: rotate(90deg);
		}
	}
	@media (max-width: 720px) {
		nav {
			display: none;
		}
		.nav {
			margin-left: 10px;
			margin-right: 10px;
			width: auto;
		}
		.gh {
			margin-left: auto;
		}
		.hero {
			padding-bottom: 250px;
		}
		.stats {
			grid-template-columns: repeat(2, 1fr);
			margin-bottom: 90px;
		}
		.grid,
		.steps {
			grid-template-columns: 1fr;
		}
		.problem,
		.demo,
		.tools,
		.how,
		.fleet {
			margin-bottom: 96px;
		}
	}
</style>
