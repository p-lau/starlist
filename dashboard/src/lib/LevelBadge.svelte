<script lang="ts">
	import { group } from './format.ts';

	let { level }: { level: string } = $props();
	const g = $derived(group(level));
</script>

<span class="badge {g}" title="Level {level}">
	<svg viewBox="0 0 10 10" aria-hidden="true">
		{#if g === 'error'}
			<path d="M5 0.8 9.2 5 5 9.2 0.8 5Z" />
		{:else if g === 'warn'}
			<path d="M5 1 9.4 9H0.6Z" />
		{:else}
			<circle cx="5" cy="5" r="3.6" />
		{/if}
	</svg>
	{level}
</span>

<style>
	.badge {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		font-size: 0.72rem;
		font-weight: 700;
		letter-spacing: 0.04em;
		padding: 2px 8px;
		border-radius: 999px;
		white-space: nowrap;
		color: var(--text);
	}
	svg {
		width: 9px;
		height: 9px;
	}
	.error {
		background: var(--error-soft);
	}
	.error svg {
		fill: var(--error);
	}
	.warn {
		background: var(--warn-soft);
	}
	.warn svg {
		fill: var(--warn);
	}
	.info {
		background: var(--info-soft);
	}
	.info svg {
		fill: var(--info);
	}
</style>
