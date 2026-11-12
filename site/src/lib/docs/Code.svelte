<script lang="ts">
	/** A code block with a title bar and a copy button. Shell blocks are split at blank
	    lines into separate commands, each with its own copy button; their "#" lines
	    are shown as captions and left out of what is copied. */
	let {
		code,
		title = '',
		lang = 'sh',
		wrap = false
	}: { code: string; title?: string; lang?: string; wrap?: boolean } = $props();
	let copied = $state(-1);

	const shell = $derived(lang === 'sh');
	const commentMark = $derived(['yaml', 'toml', 'hcl'].includes(lang) ? '#' : lang === 'groovy' ? '//' : '');
	const groups = $derived.by(() => {
		const text = code.replace(/^\n+|\s+$/g, '');
		if (!shell) return [{ note: [] as string[], body: text.split('\n') }];
		return text.split(/\n\s*\n/).map((g) => {
			const lines = g.split('\n');
			return { note: lines.filter((l) => l.startsWith('#')), body: lines.filter((l) => !l.startsWith('#')) };
		});
	});

	async function copy(i: number) {
		try {
			await navigator.clipboard.writeText(groups[i].body.join('\n'));
			copied = i;
			setTimeout(() => (copied = -1), 1600);
		} catch {
			/* clipboard blocked: the text is still selectable */
		}
	}
</script>

{#snippet button(i: number)}
	<button type="button" onclick={() => copy(i)} aria-label="Copy {groups[i].note[0]?.slice(2) || title || 'code'}">{copied === i ? 'Copied' : 'Copy'}</button>
{/snippet}

<figure class="code">
	<figcaption>
		<span class="title">{title || lang}</span>
		{#if groups.length === 1}{@render button(0)}{/if}
	</figcaption>
	{#each groups as g, gi}
		{#if groups.length > 1}
			<div class="step">
				<span>{g.note.map((n) => n.slice(2)).join(' ')}</span>
				{@render button(gi)}
			</div>
		{:else if g.note.length}
			<p class="step"><span>{g.note.map((n) => n.slice(2)).join(' ')}</span></p>
		{/if}
		<pre class:wrap><code>{#each g.body as line, i}{#if commentMark && line.trimStart().startsWith(commentMark)}<span class="c">{line}</span>{:else}{line}{/if}{#if i < g.body.length - 1}{'\n'}{/if}{/each}</code></pre>
	{/each}
</figure>

<style>
	.code {
		margin: 18px 0;
		border-radius: 12px;
		border: 1px solid var(--line);
		background: rgba(8, 10, 24, 0.8);
		overflow: hidden;
		min-width: 0;
	}
	figcaption {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		padding: 7px 10px 7px 14px;
		border-bottom: 1px solid var(--line);
		background: rgba(255, 255, 255, 0.02);
	}
	.title {
		font: 500 0.72rem var(--font-mono);
		color: var(--faint);
		letter-spacing: 0.04em;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	button {
		flex: none;
		font: 500 0.72rem var(--font-mono);
		color: var(--muted);
		background: rgba(255, 255, 255, 0.04);
		border: 1px solid var(--line);
		border-radius: 7px;
		padding: 3px 9px;
		cursor: pointer;
	}
	button:hover {
		color: var(--text);
		border-color: var(--line-strong);
	}
	pre {
		margin: 0;
		padding: 14px 16px;
		overflow-x: auto;
		font-size: 0.8rem;
		line-height: 1.65;
		color: var(--text);
		tab-size: 2;
		font-variant-ligatures: none;
	}
	pre.wrap {
		white-space: pre-wrap;
		overflow-wrap: anywhere;
	}
	.c {
		color: var(--faint);
	}
	.step {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		margin: 0;
		padding: 10px 10px 0 16px;
		font-size: 0.82rem;
		color: var(--muted);
	}
	.step + pre {
		padding-top: 6px;
	}
	pre + .step {
		border-top: 1px dashed var(--line);
	}
</style>
