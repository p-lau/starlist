<script lang="ts">
	// A tiny bar trend for a table row. The row's numbers carry the values;
	// the title gives the peak.
	let { counts, level = 'info' }: { counts: number[]; level?: string } = $props();
	const max = $derived(Math.max(1, ...counts));
	const w = 96;
	const h = 22;
	const bw = $derived(w / Math.max(1, counts.length));
</script>

<svg
	viewBox="0 0 {w} {h}"
	width={w}
	height={h}
	role="img"
	aria-label="Trend, peak {max} per step"
>
	<title>Peak {max} per step</title>
	<line x1="0" y1={h - 0.5} x2={w} y2={h - 0.5} class="base" />
	{#each counts as c, i (i)}
		{#if c > 0}
			<rect
				x={i * bw + 0.5}
				y={h - Math.max(1.5, (c / max) * (h - 2))}
				width={Math.max(1, bw - 1)}
				height={Math.max(1.5, (c / max) * (h - 2))}
				rx="1"
				class={level}
			/>
		{/if}
	{/each}
</svg>

<style>
	svg {
		display: block;
	}
	.base {
		stroke: var(--line-strong);
		stroke-width: 1;
	}
	.error {
		fill: var(--error);
	}
	.warn {
		fill: var(--warn);
	}
	.info {
		fill: var(--info);
	}
</style>
