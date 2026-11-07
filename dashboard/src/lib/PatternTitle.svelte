<script lang="ts">
	import LevelBadge from './LevelBadge.svelte';
	import StateBadge from './StateBadge.svelte';
	import type { Pattern } from './types.ts';
	import { ago } from './format.ts';

	let { p, link = true }: { p: Pattern; link?: boolean } = $props();
</script>

<div class="pt">
	<div class="top">
		<LevelBadge level={p.level} />
		{#if p.state && p.state.state !== 'open'}<StateBadge state={p.state} />{/if}
		{#if link}
			<a class="tpl mono" href="/patterns/{p.id}">{p.template}</a>
		{:else}
			<span class="tpl mono">{p.template}</span>
		{/if}
	</div>
	<div class="meta faint">
		{#if p.location}<span class="mono">{p.location}</span>{/if}
		{#if p.first_release}<span>since {p.first_release}</span>{/if}
		<span>last seen {ago(p.last_seen)}</span>
	</div>
</div>

<style>
	.pt {
		min-width: 0;
	}
	.top {
		display: flex;
		gap: 8px;
		align-items: baseline;
		min-width: 0;
	}
	.tpl {
		overflow-wrap: anywhere;
		color: var(--text);
		font-size: 0.86rem;
	}
	a.tpl:hover {
		color: var(--accent);
	}
	.meta {
		display: flex;
		flex-wrap: wrap;
		gap: 4px 14px;
		font-size: 0.78rem;
		margin-top: 4px;
	}
</style>
