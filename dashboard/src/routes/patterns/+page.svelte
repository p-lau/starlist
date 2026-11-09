<script lang="ts">
	import Tip from '#lib/Tip.svelte';
	import { get, session } from '#lib/api.svelte.ts';
	import Load from '#lib/Load.svelte';
	import PatternTitle from '#lib/PatternTitle.svelte';
	import Sparkline from '#lib/Sparkline.svelte';
	import { group, num } from '#lib/format.ts';
	import type { PatternList } from '#lib/types.ts';

	let text = $state('');
	let query = $state('');
	let level = $state('');
	let since = $state('24h');
	let stateFilter = $state('');
	let limit = $state(50);
	let list: PatternList | null = $state(null);
	let error: string | null = $state(null);

	// Search as you type, without a request per keystroke.
	let timer: ReturnType<typeof setTimeout>;
	$effect(() => {
		const t = text;
		clearTimeout(timer);
		timer = setTimeout(() => (query = t), 250);
	});

	$effect(() => {
		void session.tick;
		get<PatternList>('/patterns', { since, min_level: level, contains: query, limit, state: stateFilter }).then(
			(l) => {
				list = l;
				error = null;
			},
			(e) => (error = e.message)
		);
	});
</script>

<svelte:head><title>Patterns · starlist</title></svelte:head>

<div class="stack">
	<div class="row filters">
		<h1>Patterns<Tip text="Every kind of log line starlist has seen. Parts that change, like ids and numbers, become placeholders, so similar lines group together. Click one for its details." /></h1>
		<input type="search" placeholder="Search templates" bind:value={text} aria-label="Search templates" />
		<select bind:value={level} aria-label="Level">
			<option value="">All levels</option>
			<option value="INFO">Info and up</option>
			<option value="WARN">Warnings and up</option>
			<option value="ERROR">Errors only</option>
		</select>
		<select bind:value={stateFilter} aria-label="State">
			<option value="">Any state</option>
			<option value="open">Open</option>
			<option value="regressed">Regressed</option>
			<option value="resolved">Resolved</option>
			<option value="ignored">Ignored</option>
		</select>
		<select bind:value={since} aria-label="Period">
			<option value="1h">Last hour</option>
			<option value="24h">Last 24 hours</option>
			<option value="7d">Last 7 days</option>
		</select>
	</div>

	<section class="card flush">
		{#if !list}
			<div class="pad"><Load {error} loading={!list} /></div>
		{:else if list.patterns.length === 0}
			<p class="pad muted">No patterns match.</p>
		{:else}
			<table>
				<thead>
					<tr>
						<th>Pattern</th>
						<th class="num">Events</th>
						<th class="trend">Trend</th>
					</tr>
				</thead>
				<tbody>
					{#each list.patterns as p (p.id)}
						<tr>
							<td><PatternTitle {p} /></td>
							<td class="num">{num(p.count)}</td>
							<td class="trend">{#if p.trend}<Sparkline counts={p.trend} level={group(p.level)} />{/if}</td>
						</tr>
					{/each}
				</tbody>
			</table>
			<div class="pad row foot">
				<span class="faint">{list.patterns.length} of {list.total}</span>
				{#if list.total > list.patterns.length}
					<button onclick={() => (limit += 50)}>Show more</button>
				{/if}
			</div>
		{/if}
	</section>
</div>

<style>
	.filters input {
		flex: 1 1 200px;
	}
	.filters h1 {
		margin-right: 8px;
	}
	.flush {
		padding: 0;
		overflow: hidden;
	}
	.pad {
		padding: 14px 18px;
		margin: 0;
	}
	th:first-child,
	td:first-child {
		padding-left: 18px;
	}
	th:last-child,
	td:last-child {
		padding-right: 18px;
	}
	.trend {
		width: 110px;
	}
	.foot {
		justify-content: space-between;
	}
	@media (max-width: 560px) {
		.trend {
			display: none;
		}
	}
</style>
