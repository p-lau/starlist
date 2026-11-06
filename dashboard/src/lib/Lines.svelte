<script lang="ts">
	import type { Line } from './types.ts';
	import { duration } from './format.ts';

	// Raw log lines; `anchor` is highlighted, `offsets` shows time from the first line.
	let { lines, anchor = null, offsets = false }: { lines: Line[]; anchor?: number | null; offsets?: boolean } =
		$props();
</script>

<ol class="lines">
	{#each lines as l (l.id)}
		<li class:anchor={l.id === anchor}>
			<span class="t faint mono">
				{offsets && l.offset_ms !== undefined
					? `+${duration(l.offset_ms)}`
					: new Date(l.at).toLocaleTimeString(undefined, { hour12: false })}
			</span>
			<a class="p faint mono" href="/patterns/{l.pattern_id}" title="Pattern {l.pattern_id}">#</a>
			<pre>{l.text}</pre>
		</li>
	{/each}
</ol>

<style>
	.lines {
		list-style: none;
		margin: 0;
		padding: 0;
		border: 1px solid var(--line);
		border-radius: 8px;
		overflow: hidden;
	}
	li {
		display: grid;
		grid-template-columns: 76px 14px minmax(0, 1fr);
		gap: 6px;
		padding: 3px 10px;
		border-top: 1px solid var(--line);
		align-items: baseline;
	}
	li:first-child {
		border-top: 0;
	}
	li.anchor {
		background: var(--error-soft);
		box-shadow: inset 3px 0 0 var(--error);
	}
	.t {
		font-size: 0.75rem;
		white-space: nowrap;
	}
	.p {
		font-size: 0.75rem;
	}
	pre {
		padding: 0;
		background: none;
		border: 0;
		font-size: 0.8rem;
	}
</style>
