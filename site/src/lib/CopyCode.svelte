<script lang="ts">
	let { code, label = '' }: { code: string; label?: string } = $props();
	let copied = $state(false);

	async function copy() {
		try {
			await navigator.clipboard.writeText(code);
			copied = true;
			setTimeout(() => (copied = false), 1600);
		} catch {
			/* clipboard blocked: the text is still selectable */
		}
	}
</script>

<div class="cc">
	{#if label}<span class="label">{label}</span>{/if}
	<pre><code><span class="prompt" aria-hidden="true">$</span> {code}</code></pre>
	<button onclick={copy} aria-label="Copy command">{copied ? 'Copied' : 'Copy'}</button>
</div>

<style>
	.cc {
		position: relative;
		border-radius: 12px;
		border: 1px solid var(--line);
		background: rgba(8, 10, 24, 0.75);
	}
	.label {
		display: block;
		padding: 10px 16px 0;
		font: 500 0.72rem var(--font-mono);
		color: var(--faint);
		text-transform: uppercase;
		letter-spacing: 0.1em;
	}
	pre {
		margin: 0;
		padding: 10px 88px 14px 16px;
		overflow-x: auto;
		font-size: 0.85rem;
		color: var(--text);
	}
	.prompt {
		color: var(--star);
		user-select: none;
	}
	button {
		position: absolute;
		right: 10px;
		bottom: 9px;
		font: 500 0.75rem var(--font-mono);
		color: var(--muted);
		background: rgba(255, 255, 255, 0.04);
		border: 1px solid var(--line);
		border-radius: 8px;
		padding: 5px 10px;
		cursor: pointer;
	}
	button:hover {
		color: var(--text);
		border-color: var(--line-strong);
	}
</style>
