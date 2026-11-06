<script lang="ts">
	import { page } from '$app/state';
	import { get, session } from '#lib/api.svelte.ts';
	import Lines from '#lib/Lines.svelte';
	import Load from '#lib/Load.svelte';
	import { duration, when } from '#lib/format.ts';
	import type { RequestTrace } from '#lib/types.ts';

	const id = $derived(page.params.id ?? '');
	let trace: RequestTrace | null = $state(null);
	let error: string | null = $state(null);

	$effect(() => {
		void session.tick;
		get<RequestTrace>(`/requests/${encodeURIComponent(id)}`).then(
			(t) => {
				trace = t;
				error = null;
			},
			(e) => (error = e.message)
		);
	});
</script>

<svelte:head><title>Request {id} · starlist</title></svelte:head>

<div class="stack">
	<div>
		<h1>Request <span class="mono">{id}</span></h1>
		{#if trace}
			<p class="muted">
				{trace.events} events over {duration(trace.duration_ms)}, starting {when(trace.started)}.
				{#if trace.truncated}Only the first {trace.events} are shown.{/if}
			</p>
		{/if}
	</div>
	{#if trace}
		<Lines lines={trace.lines} offsets />
	{:else}
		<Load {error} loading={!trace} />
	{/if}
</div>
