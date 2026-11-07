<script lang="ts">
	import { page } from '$app/state';
	import { get, post, refresh, session } from '#lib/api.svelte.ts';
	import StateBadge from '#lib/StateBadge.svelte';
	import LevelBadge from '#lib/LevelBadge.svelte';
	import LevelChart from '#lib/LevelChart.svelte';
	import Lines from '#lib/Lines.svelte';
	import Sparkline from '#lib/Sparkline.svelte';
	import Load from '#lib/Load.svelte';
	import { ago, approx, num, when } from '#lib/format.ts';
	import type { Context, FieldSeries, Marker, PatternDetail, Timeline, Verify } from '#lib/types.ts';

	const id = $derived(page.params.id ?? '');
	let p: PatternDetail | null = $state(null);
	let timeline: Timeline | null = $state(null);
	let markers: Marker[] = $state([]);
	let error: string | null = $state(null);
	let range = $state('24h');

	// p95 per 6 hours over a week, per field.
	let trends: Record<string, number[]> = $state({});
	$effect(() => {
		const fields = p?.fields.map((f) => f.field) ?? [];
		const pid = p?.id;
		if (!pid || !fields.length) return;
		Promise.all(
			fields.map((f) =>
				get<FieldSeries>(`/fields/${encodeURIComponent(f)}`, { pattern_id: pid, since: '7d', step: '6h' })
			)
		).then(
			(all) => (trends = Object.fromEntries(all.map((s) => [s.field, s.points.map((x) => x.p95 ?? 0)]))),
			() => (trends = {})
		);
	});

	let ctx: Context | null = $state(null);
	let ctxError: string | null = $state(null);

	let since = $state('');
	let verdict: Verify | null = $state(null);
	let verifyError: string | null = $state(null);

	$effect(() => {
		void session.tick;
		const pid = id;
		Promise.all([
			get<PatternDetail>(`/patterns/${encodeURIComponent(pid)}`),
			get<Timeline>('/timeline', { since: range, pattern_id: pid }),
			get<{ markers: Marker[] }>('/markers', { limit: 50 })
		]).then(
			([d, t, m]) => {
				p = d;
				timeline = t;
				markers = m.markers;
				error = null;
				if (!since && m.markers.length) since = m.markers[0].name;
			},
			(e) => (error = e.message)
		);
	});

	async function loadContext() {
		try {
			ctx = await get<Context>('/context', { pattern_id: id, before: 15, after: 10 });
			ctxError = null;
		} catch (e) {
			ctxError = (e as Error).message;
		}
	}

	async function verify() {
		try {
			verdict = await get<Verify>(`/patterns/${encodeURIComponent(id)}/verify`, { since });
			verifyError = null;
		} catch (e) {
			verifyError = (e as Error).message;
		}
	}

	let stateNote = $state('');
	let stateError: string | null = $state(null);
	let busy = $state(false);
	async function setState(state: string, extra: Record<string, unknown> = {}) {
		busy = true;
		try {
			p = await post<PatternDetail>(`/patterns/${encodeURIComponent(id)}/state`, {
				state,
				note: stateNote.trim() || null,
				...extra
			});
			stateError = null;
			stateNote = '';
			refresh();
		} catch (e) {
			stateError = (e as Error).message;
		} finally {
			busy = false;
		}
	}

	const GOOD = new Set(['fixed', 'reduced']);
	const BAD = new Set(['worse', 'still_occurring', 'appeared_after']);
</script>

<svelte:head><title>{p ? p.template.slice(0, 60) : 'Pattern'} · starlist</title></svelte:head>

{#if !p}
	<Load {error} loading={!p} />
{:else}
	<div class="stack">
		<section class="card">
			<div class="row">
				<LevelBadge level={p.level} />
				<StateBadge state={p.state} />
				<span class="faint mono id">{p.id}</span>
			</div>
			<h1 class="mono tpl">{p.template}</h1>
			<dl class="facts">
				{#if p.exception}<div><dt>Exception</dt><dd class="mono">{p.exception}</dd></div>{/if}
				{#if p.location}<div><dt>Location</dt><dd class="mono">{p.location}</dd></div>{/if}
				<div><dt>First seen</dt><dd title={when(p.first_seen)}>{ago(p.first_seen)}{#if p.first_release}{' in '}{p.first_release}{/if}</dd></div>
				<div><dt>Last seen</dt><dd title={when(p.last_seen)}>{ago(p.last_seen)}</dd></div>
				<div><dt>Last hour</dt><dd>{num(p.count_last_hour)}</dd></div>
				<div><dt>Last 24h</dt><dd>{num(p.count_last_24h)}</dd></div>
				<div><dt>All time</dt><dd>{num(p.total)}</dd></div>
			</dl>
		</section>

		<section class="card stack">
			<h2>Issue state</h2>
			{#if p.state.state === 'regressed'}
				<p class="error-box">
					This was resolved {ago(p.state.resolved_at)} and came back {ago(p.state.regressed_at)}{#if p.state.regressed_in}{' in '}{p.state.regressed_in}{/if}.
				</p>
			{:else if p.state.state !== 'open'}
				<p class="muted">
					{#if p.state.state === 'ignored'}
						Left out of what's new and alerts{#if p.state.until}{' until '}{when(p.state.until)}{/if}{#if p.state.remaining}{' for '}{p.state.remaining} more events{/if}.
					{:else if p.state.state === 'resolved_next'}
						Resolved from the next deploy on. If it occurs after that, it shows as regressed.
					{:else}
						Resolved {ago(p.state.resolved_at)}. If it occurs again, it shows as regressed.
					{/if}
					{#if p.state.note}<br />Note: {p.state.note}{/if}
				</p>
			{/if}
			<div class="row">
				<input class="note" bind:value={stateNote} placeholder="Note (optional)" aria-label="Note" />
			</div>
			<div class="row">
				{#if p.state.state !== 'resolved'}
					<button class="primary" disabled={busy} onclick={() => setState('resolved')}>Resolve</button>
				{/if}
				{#if p.state.state !== 'resolved_next'}
					<button disabled={busy} onclick={() => setState('resolved_next')}>Resolve in next release</button>
				{/if}
				{#if p.state.state !== 'ignored'}
					<button disabled={busy} onclick={() => setState('ignored', { ignore_for: '24h' })}>Ignore for 24h</button>
					<button disabled={busy} onclick={() => setState('ignored')}>Ignore</button>
				{/if}
				{#if p.state.state !== 'open'}
					<button disabled={busy} onclick={() => setState('open')}>Reopen</button>
				{/if}
			</div>
			{#if stateError}<div class="error-box">{stateError}</div>{/if}
		</section>

		<section class="card">
			<div class="head">
				<h2>Occurrences</h2>
				<select bind:value={range} aria-label="Period">
					<option value="6h">6 hours</option>
					<option value="24h">24 hours</option>
					<option value="7d">7 days</option>
				</select>
			</div>
			{#if timeline}<LevelChart data={timeline} height={170} />{/if}
		</section>

		<section class="card stack">
			<h2>Did a fix work?</h2>
			<div class="row">
				<label for="since" class="muted">Compare before and after</label>
				<select id="since" bind:value={since}>
					{#each markers as m (m.name + m.at)}<option value={m.name}>{m.name} ({ago(m.at)})</option>{/each}
					<option value="1h">1 hour ago</option>
					<option value="6h">6 hours ago</option>
				</select>
				<button class="primary" onclick={verify} disabled={!since}>Check</button>
			</div>
			{#if verifyError}<div class="error-box">{verifyError}</div>{/if}
			{#if verdict}
				<div class="verdict" class:good={GOOD.has(verdict.verdict)} class:bad={BAD.has(verdict.verdict)}>
					<strong>{verdict.verdict.replaceAll('_', ' ')}</strong>
					<span>{verdict.explanation}</span>
					{#if verdict.before}
						<span class="faint">Before: {num(verdict.before.count)} ({verdict.before.per_hour}/h). After: {num(verdict.after.count)} ({verdict.after.per_hour}/h).</span>
					{/if}
					{#if verdict.caveat}<span class="faint">{verdict.caveat}</span>{/if}
				</div>
			{/if}
		</section>

		{#if p.fields?.length}
			<section class="card">
				<h2>Numbers in its lines, last 24h</h2>
				<div class="scroll">
					<table>
						<thead>
							<tr>
								<th>Field</th>
								<th class="num">Count</th>
								<th class="num">p50</th>
								<th class="num">p95</th>
								<th class="num">p99</th>
								<th class="num">Max</th>
								<th>p95, 7 days</th>
							</tr>
						</thead>
						<tbody>
							{#each p.fields as f (f.field)}
								<tr>
									<td class="mono">{f.field}</td>
									<td class="num">{num(f.count)}</td>
									<td class="num">{approx(f.p50)}</td>
									<td class="num">{approx(f.p95)}</td>
									<td class="num">{approx(f.p99)}</td>
									<td class="num">{num(f.max)}</td>
									<td>{#if trends[f.field]}<Sparkline counts={trends[f.field]} label="p95 in 6 hours" />{/if}</td>
								</tr>
							{/each}
						</tbody>
					</table>
				</div>
				<p class="faint small">Percentiles are within about 9%; count and max are exact.</p>
			</section>
		{/if}

		{#if p.locations.length > 1}
			<section class="card">
				<h2>Raised from</h2>
				<table>
					<tbody>
						{#each p.locations as l (l.location)}
							<tr><td class="mono">{l.location}</td><td class="num">{num(l.count)}</td></tr>
						{/each}
					</tbody>
				</table>
			</section>
		{/if}

		<section class="card stack">
			<div class="head">
				<h2>Recent samples</h2>
				<div class="row">
					{#if p.example_request}
						<a href="/requests/{encodeURIComponent(p.example_request)}">Follow a request</a>
					{/if}
					<button onclick={loadContext}>Show surrounding lines</button>
				</div>
			</div>
			{#if ctxError}<div class="error-box">{ctxError}</div>{/if}
			{#if ctx}
				<Lines lines={ctx.lines} anchor={ctx.anchor.id} />
			{/if}
			{#each p.samples as s (s.at)}
				<div>
					<div class="faint small" title={when(s.at)}>{ago(s.at)}</div>
					<pre>{s.text}</pre>
				</div>
			{/each}
		</section>
	</div>
{/if}

<style>
	.id {
		font-size: 0.78rem;
	}
	.tpl {
		font-size: 1rem;
		font-weight: 600;
		margin: 10px 0 14px;
		overflow-wrap: anywhere;
	}
	.facts {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
		gap: 10px 18px;
		margin: 0;
	}
	dt {
		font-size: 0.75rem;
		color: var(--muted);
	}
	dd {
		margin: 0;
		overflow-wrap: anywhere;
	}
	.head {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 10px;
		flex-wrap: wrap;
		margin-bottom: 8px;
	}
	.verdict {
		display: grid;
		gap: 4px;
		padding: 10px 14px;
		border-radius: 8px;
		border: 1px solid var(--line-strong);
	}
	.verdict strong {
		text-transform: capitalize;
	}
	.verdict.good {
		border-color: var(--good);
		background: var(--good-soft);
	}
	.verdict.bad {
		border-color: var(--error);
		background: var(--error-soft);
	}
	.note {
		flex: 1 1 260px;
	}
	.error-box {
		margin: 0;
	}
	.small {
		font-size: 0.78rem;
		margin-bottom: 4px;
	}
	.scroll {
		overflow-x: auto;
	}
</style>
