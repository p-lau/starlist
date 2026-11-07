<script lang="ts">
	// Shows a load error, or a quiet placeholder before the first answer.
	let { error, loading }: { error: string | null; loading: boolean } = $props();
</script>

{#if error}
	<div class="error-box">{error}</div>
{:else if loading}
	<p class="faint">Loading…</p>
{/if}
