<script lang="ts">
	import Tip from '#lib/Tip.svelte';
	import { get, post, refresh, session } from '#lib/api.svelte.ts';
	import Load from '#lib/Load.svelte';
	import PatternTitle from '#lib/PatternTitle.svelte';
	import { ago, num, rate, when } from '#lib/format.ts';
	import type { Change, Compare, Marker, Release } from '#lib/types.ts';

	let markers: Marker[] | null = $state(null);
	let selected: string | null = $state(null);
	let diff: Compare | null = $state(null);
	let error: string | null = $state(null);
	let diffError: string | null = $state(null);
	let rel: Release | null = $state(null);

	let name = $state('');
	let note = $state('');
	let deploy = $state(true);
	let saving = $state(false);
	let saveError: string | null = $state(null);

	const KIND: Record<Change['kind'], string> = {
		new_after: 'New',
		increased: 'Increased',
		stopped: 'Stopped',
		decreased: 'Decreased'
	};

	$effect(() => {
		void session.tick;
		get<{ markers: Marker[] }>('/markers', { limit: 100 }).then(
			(m) => {
				markers = m.markers;
				error = null;
				if (selected === null && m.markers.length) selected = m.markers[0].name;
			},
			(e) => (error = e.message)
		);
	});

	$effect(() => {
		void session.tick;
		const at = selected;
		if (!at) return;
		get<Compare>('/compare', { at, limit: 100 }).then(
			(d) => {
				diff = d;
				diffError = null;
			},
			(e) => {
				diff = null;
				diffError = e.message;
			}
		);
	});

	// A deploy also gets its release report card.
	$effect(() => {
		void session.tick;
		const m = markers?.find((x) => x.name === selected);
		if (!m || m.kind !== 'deploy') {
			rel = null;
			return;
		}
		get<Release>(`/releases/${encodeURIComponent(m.name)}`).then(
			(r) => (rel = r),
			() => (rel = null)
		);
	});

	async function add(e: SubmitEvent) {
		e.preventDefault();
		saving = true;
		try {
			const m = await post<Marker>('/markers', { name: name.trim(), note: note.trim() || null, deploy });
			name = '';
			note = '';
			selected = m.name;
			saveError = null;
			refresh();
		} catch (err) {
			saveError = (err as Error).message;
		} finally {
			saving = false;
		}
	}
</script>

<svelte:head><title>Deploys · starlist</title></svelte:head>

<div class="layout">
	<aside class="stack">
		<h1>Deploys and markers<Tip text="A deploy is a release going live. A marker is any moment you want to compare around, like a config change. Pick one to see which patterns appeared, grew, shrank or stopped after it." /></h1>
		{#if !markers}
			<Load {error} loading={!markers} />
		{:else if markers.length === 0}
			<p class="muted">
				No deploys yet. starlist records one when a line like <code>Starting api version 1.4.2</code> appears,
				when it starts with <code>STARLIST_VERSION</code>, or when you add one below.
			</p>
		{:else}
			<ul class="markers">
				{#each markers as m (m.name + m.at)}
					<li>
						<button class:on={selected === m.name} onclick={() => (selected = m.name)}>
							<span class="name mono">{m.name}</span>
							<span class="faint small" title={when(m.at)}>{m.kind === 'deploy' ? 'Deploy' : 'Marker'} · {ago(m.at)}</span>
							{#if m.note}<span class="muted small">{m.note}</span>{/if}
						</button>
					</li>
				{/each}
			</ul>
		{/if}
		<form class="card stack add" onsubmit={add}>
			<h3>Add a marker</h3>
			<input bind:value={name} placeholder="Name, like v1.4.3 or fix-currency" aria-label="Name" required />
			<input bind:value={note} placeholder="Note (optional)" aria-label="Note" />
			<label class="row small"><input type="checkbox" bind:checked={deploy} /> A release going live</label>
			{#if saveError}<div class="error-box">{saveError}</div>{/if}
			<button class="primary" type="submit" disabled={saving || !name.trim()}>Add at the current time</button>
		</form>
	</aside>

	<div class="stack">
	{#if rel}
		<section class="card stack">
			<div>
				<h2>Release {rel.name}{#if rel.env}<span class="faint">{' · '}{rel.env}</span>{/if}</h2>
				<p class="faint small">
					{rel.current ? `Live since ${when(rel.at)}` : `Live ${when(rel.at)} to ${when(rel.until)}`}
				</p>
			</div>
			<div class="stats">
				<div><span class="faint small">Errors per hour</span><b>{rel.errors_per_hour}</b>
					{#if rel.previous_errors_per_hour !== null}<span class="faint small">vs {rel.previous_errors_per_hour} in {rel.previous}</span>{/if}
				</div>
				<div><span class="faint small">Events per hour</span><b>{num(rel.events_per_hour)}</b></div>
				<div><span class="faint small">New patterns</span><b>{rel.new.length}</b></div>
				<div><span class="faint small">Regressed</span><b>{rel.regressed.length}</b></div>
				<div><span class="faint small">Resolved</span><b>{rel.resolved.length}</b></div>
			</div>
			{#each [{ t: 'Regressed in this release', l: rel.regressed }, { t: 'First seen in this release', l: rel.new }, { t: 'Resolved in this release', l: rel.resolved }] as g (g.t)}
				{#if g.l.length}
					<div>
						<h3>{g.t}</h3>
						<ul class="plain">
							{#each g.l.slice(0, 10) as p (p.id)}<li><PatternTitle {p} /></li>{/each}
						</ul>
						{#if g.l.length > 10}<p class="faint small">and {g.l.length - 10} more</p>{/if}
					</div>
				{/if}
			{/each}
		</section>
	{/if}
	<section class="card">
		{#if diffError}
			<div class="error-box">{diffError}</div>
		{:else if diff}
			<h2>Around {diff.split.marker ?? when(diff.split.at)}</h2>
			<p class="muted">{diff.summary}</p>
			<p class="faint small">
				{num(diff.before.events)} events in the {ago(diff.before.from).replace(' ago', '')} before,
				{num(diff.after.events)} since.
			</p>
			{#if diff.changes.length === 0}
				<p class="muted">No pattern changed noticeably.</p>
			{:else}
				<ul class="changes">
					{#each diff.changes as c (c.pattern.id)}
						<li>
							<span class="kind {c.kind}">{KIND[c.kind]}</span>
							<PatternTitle p={c.pattern} />
							<span class="nums small">
								<span>{rate(c.before.per_hour)} → <b>{rate(c.after.per_hour)}</b></span>
								<span class="faint">{num(c.before.count)} → {num(c.after.count)}</span>
							</span>
						</li>
					{/each}
				</ul>
			{/if}
		{:else if markers?.length}
			<p class="faint">Loading…</p>
		{:else}
			<p class="muted">Pick a deploy to see what changed around it.</p>
		{/if}
	</section>
	</div>
</div>

<style>
	.layout {
		display: grid;
		grid-template-columns: 300px minmax(0, 1fr);
		gap: 20px;
		align-items: start;
	}
	.markers {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 6px;
	}
	.markers button {
		width: 100%;
		text-align: left;
		display: grid;
		gap: 2px;
	}
	.markers button.on {
		border-color: var(--accent);
		background: var(--accent-soft);
	}
	.name {
		font-weight: 600;
	}
	.small {
		font-size: 0.8rem;
	}
	.add input[type='checkbox'] {
		margin: 0;
	}
	.stats {
		display: grid;
		grid-template-columns: repeat(auto-fill, minmax(130px, 1fr));
		gap: 12px;
	}
	.stats div {
		display: grid;
	}
	.stats b {
		font-size: 1.3rem;
		font-variant-numeric: tabular-nums;
	}
	.plain {
		list-style: none;
		margin: 6px 0 0;
		padding: 0;
		display: grid;
		gap: 8px;
	}
	.changes {
		list-style: none;
		margin: 0;
		padding: 0;
	}
	.changes li {
		display: grid;
		grid-template-columns: 86px minmax(0, 1fr) auto;
		gap: 12px;
		padding: 10px 0;
		border-top: 1px solid var(--line);
	}
	.kind {
		font-size: 0.75rem;
		font-weight: 700;
		text-transform: uppercase;
		letter-spacing: 0.04em;
		padding-top: 2px;
	}
	.kind.new_after,
	.kind.increased {
		color: var(--error);
	}
	.kind.stopped,
	.kind.decreased {
		color: var(--good);
	}
	.nums {
		display: grid;
		text-align: right;
		font-variant-numeric: tabular-nums;
	}
	@media (max-width: 820px) {
		.layout {
			grid-template-columns: minmax(0, 1fr);
		}
		.changes li {
			grid-template-columns: minmax(0, 1fr) auto;
		}
		.kind {
			grid-column: 1 / -1;
		}
	}
</style>
