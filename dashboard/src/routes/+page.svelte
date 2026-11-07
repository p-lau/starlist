<script lang="ts">
	import Tip from '#lib/Tip.svelte';
	import { get, session } from '#lib/api.svelte.ts';
	import LevelChart from '#lib/LevelChart.svelte';
	import PatternTitle from '#lib/PatternTitle.svelte';
	import Load from '#lib/Load.svelte';
	import { num, rate } from '#lib/format.ts';
	import type { Finding, Status, Timeline, WhatsNew } from '#lib/types.ts';

	const RANGES = [
		{ v: '1h', label: '1 hour' },
		{ v: '6h', label: '6 hours' },
		{ v: '24h', label: '24 hours' },
		{ v: '7d', label: '7 days' }
	];
	const WINDOWS = ['15m', '1h', '6h', '24h'];
	const KIND: Record<Finding['kind'], string> = {
		new: 'New',
		regressed: 'Regressed',
		returning: 'Returning',
		spike: 'Spiking',
		rising: 'Rising',
		gone_quiet: 'Gone quiet'
	};

	let range = $state('24h');
	let window = $state('1h');
	let minLevel = $state('WARN');
	let status: Status | null = $state(null);
	let timeline: Timeline | null = $state(null);
	let news: WhatsNew | null = $state(null);
	let error: string | null = $state(null);

	$effect(() => {
		void session.tick;
		const r = range;
		const w = window;
		const l = minLevel;
		Promise.all([
			get<Status>('/status'),
			get<Timeline>('/timeline', { since: r }),
			get<WhatsNew>('/whats-new', { window: w, min_level: l, limit: 50 })
		]).then(
			([s, t, n]) => {
				status = s;
				timeline = t;
				news = n;
				error = null;
			},
			(e) => (error = e.message)
		);
	});
</script>

<svelte:head><title>Overview · starlist</title></svelte:head>

<div class="stack">
	<section class="tiles">
		<div class="card tile">
			<span class="label">Events, last hour<Tip text="Log lines read in the last 60 minutes, at every level." /></span>
			<span class="value">{num(status?.last_hour.events)}</span>
		</div>
		<div class="card tile" class:alarm={(status?.last_hour.errors ?? 0) > 0}>
			<span class="label">Errors, last hour</span>
			<span class="value">{num(status?.last_hour.errors)}</span>
		</div>
		<div class="card tile">
			<span class="label">Patterns<Tip text="Distinct kinds of log line. Lines that differ only in ids, numbers and the like share one template and count as one pattern." /></span>
			<span class="value">{num(status?.patterns)}</span>
		</div>
		<div class="card tile">
			<span class="label">Live release<Tip text="The latest deploy starlist knows of. It records one from a startup line naming a version, from STARLIST_VERSION, or when you add one under Deploys." /></span>
			<span class="value mono small">{status?.current_release?.name ?? '—'}</span>
		</div>
	</section>

	<section class="card">
		<div class="head">
			<h2>Log volume by level<Tip text="How many lines arrived over time, stacked by level. Pick the period on the right." /></h2>
			<div class="seg" role="group" aria-label="Period">
				{#each RANGES as r (r.v)}
					<button class:on={range === r.v} aria-pressed={range === r.v} onclick={() => (range = r.v)}>{r.v}</button>
				{/each}
			</div>
		</div>
		{#if timeline}
			<LevelChart data={timeline} />
		{:else}
			<Load {error} loading={!timeline} />
		{/if}
	</section>

	<section class="card">
		<div class="head">
			<div>
				<h2>What's new<Tip text="Patterns that changed in the chosen window compared with the day before. New: never seen before. Regressed: was resolved and came back. Returning: back after a long quiet. Spiking or Rising: busier than usual. Gone quiet: stopped. The numbers on the right are the count in the window and its rate per hour against the usual rate." /></h2>
				{#if news}<p class="muted sub">{news.summary}</p>{/if}
			</div>
			<div class="row">
				<label class="faint" for="win">Last</label>
				<select id="win" bind:value={window}>
					{#each WINDOWS as w (w)}<option value={w}>{w}</option>{/each}
				</select>
				<label class="faint" for="lvl">Level</label>
				<select id="lvl" bind:value={minLevel}>
					<option value="">All</option>
					<option value="WARN">Warnings and up</option>
					<option value="ERROR">Errors only</option>
				</select>
			</div>
		</div>
		{#if error && !news}
			<Load {error} loading={false} />
		{:else if news && news.findings.length === 0}
			<p class="empty">Nothing new. Every pattern is within its usual rate compared with the day before.</p>
		{:else if news}
			<ul class="findings">
				{#each news.findings as f (f.pattern.id)}
					<li class="finding">
						<span class="kind {f.kind}">{KIND[f.kind]}</span>
						<div class="body">
							<PatternTitle p={f.pattern} />
							{#if f.note}<p class="note muted">{f.note}</p>{/if}
							{#if f.sample}
								<details>
									<summary>Latest sample</summary>
									<pre>{f.sample.text}</pre>
								</details>
							{/if}
						</div>
						<div class="nums">
							<b>{num(f.count_window)}</b>
							<span class="faint">{rate(f.per_hour_window)} vs {rate(f.per_hour_baseline)}</span>
						</div>
					</li>
				{/each}
			</ul>
		{/if}
	</section>
</div>

<style>
	.tiles {
		display: grid;
		grid-template-columns: repeat(4, minmax(0, 1fr));
		gap: 12px;
	}
	.tile {
		display: grid;
		gap: 2px;
	}
	.label {
		color: var(--muted);
		font-size: 0.8rem;
	}
	.value {
		font-size: 1.6rem;
		font-weight: 650;
		font-variant-numeric: tabular-nums;
		overflow-wrap: anywhere;
	}
	.value.small {
		font-size: 1.15rem;
		padding-top: 6px;
	}
	.tile.alarm {
		border-color: var(--error);
	}
	.head {
		display: flex;
		justify-content: space-between;
		align-items: flex-start;
		gap: 12px;
		flex-wrap: wrap;
		margin-bottom: 12px;
	}
	.sub {
		margin: 4px 0 0;
		font-size: 0.85rem;
	}
	.seg {
		display: inline-flex;
		border: 1px solid var(--line-strong);
		border-radius: 8px;
		overflow: hidden;
	}
	.seg button {
		border: 0;
		border-radius: 0;
		padding: 4px 10px;
		background: transparent;
	}
	.seg button.on {
		background: var(--accent-soft);
		color: var(--text);
		font-weight: 600;
	}
	.findings {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.finding {
		display: grid;
		grid-template-columns: 96px minmax(0, 1fr) auto;
		gap: 12px;
		padding: 12px 0;
		border-top: 1px solid var(--line);
	}
	.finding:first-child {
		border-top: 0;
	}
	.kind {
		font-size: 0.75rem;
		font-weight: 700;
		text-transform: uppercase;
		letter-spacing: 0.04em;
		padding-top: 2px;
	}
	.kind.new,
	.kind.regressed,
	.kind.returning {
		color: var(--error);
	}
	.kind.spike,
	.kind.rising {
		color: var(--warn);
	}
	.kind.gone_quiet {
		color: var(--good);
	}
	.note {
		margin: 6px 0 0;
		font-size: 0.82rem;
	}
	details {
		margin-top: 6px;
	}
	summary {
		cursor: pointer;
		font-size: 0.8rem;
		color: var(--muted);
	}
	details pre {
		margin-top: 6px;
	}
	.nums {
		display: grid;
		text-align: right;
		font-size: 0.8rem;
		font-variant-numeric: tabular-nums;
	}
	.nums b {
		font-size: 1.1rem;
	}
	.empty {
		color: var(--muted);
		margin: 0;
	}
	@media (max-width: 760px) {
		.tiles {
			grid-template-columns: repeat(2, minmax(0, 1fr));
		}
		.finding {
			grid-template-columns: minmax(0, 1fr) auto;
		}
		.kind {
			grid-column: 1 / -1;
		}
	}
</style>
