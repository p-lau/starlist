<script lang="ts">
	import type { PatternState } from './types.ts';
	import { ago } from './format.ts';

	let { state }: { state: PatternState } = $props();
	const LABEL: Record<PatternState['state'], string> = {
		open: 'Open',
		resolved: 'Resolved',
		resolved_next: 'Resolved next release',
		ignored: 'Ignored',
		regressed: 'Regressed'
	};
	const title = $derived(
		state.state === 'regressed'
			? `Resolved ${ago(state.resolved_at)}, back ${ago(state.regressed_at)}${state.regressed_in ? ` in ${state.regressed_in}` : ''}`
			: state.state === 'ignored' && state.until
				? `Ignored until ${new Date(state.until).toLocaleString()}`
				: (state.note ?? '')
	);
</script>

<span class="st {state.state}" {title}>{LABEL[state.state]}</span>

<style>
	.st {
		font-size: 0.72rem;
		font-weight: 600;
		padding: 1px 8px;
		border-radius: 999px;
		border: 1px solid var(--line-strong);
		color: var(--muted);
		white-space: nowrap;
	}
	.resolved,
	.resolved_next {
		border-color: var(--good);
		color: var(--good);
	}
	.regressed {
		border-color: var(--error);
		color: var(--error);
		font-weight: 700;
	}
</style>
