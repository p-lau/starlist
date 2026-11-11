<script lang="ts">
	type Line = { t: string; c?: 'k' | 's' | 'n' | 'g' | 'b' | 'm' | 'p' };
	type Demo = { tool: string; ask: string; out: Line[][] };

	const demos: Demo[] = [
		{
			tool: 'whats_new',
			ask: 'What changed in the checkout service in the last hour?',
			out: [
				[{ t: '{' }],
				[{ t: '  "window": ', c: 'k' }, { t: '"last 1h (since 09:02)"', c: 's' }, { t: ',' }],
				[{ t: '  "summary": ', c: 'k' }, { t: '"1 new error, 1 spike, 1 went quiet"', c: 's' }, { t: ',' }],
				[{ t: '  "findings": [', c: 'k' }],
				[{ t: '    { ' }, { t: '"kind": ', c: 'k' }, { t: '"new"', c: 'b' }, { t: ', ' }, { t: '"level": ', c: 'k' }, { t: '"ERROR"', c: 'b' }, { t: ',' }],
				[{ t: '      "pattern_id": ', c: 'k' }, { t: '"p_7c1e9a"', c: 'p' }, { t: ',' }],
				[{ t: '      "template": ', c: 'k' }, { t: '"charge failed for order <*>: <str> | KeyError"', c: 's' }, { t: ',' }],
				[{ t: '      "first_release": ', c: 'k' }, { t: '"v1.4.2"', c: 's' }, { t: ', ' }, { t: '"count_in_window": ', c: 'k' }, { t: '412', c: 'n' }, { t: ' },' }],
				[{ t: '    { ' }, { t: '"kind": ', c: 'k' }, { t: '"spike"', c: 'm' }, { t: ', ' }, { t: '"level": ', c: 'k' }, { t: '"WARN"', c: 'm' }, { t: ',' }],
				[{ t: '      "template": ', c: 'k' }, { t: '"retrying payment provider, attempt <n>"', c: 's' }, { t: ',' }],
				[{ t: '      "per_hour": ', c: 'k' }, { t: '{ "window": ' }, { t: '380', c: 'n' }, { t: ', "baseline": ' }, { t: '22', c: 'n' }, { t: ' } }' }],
				[{ t: '  ],' }],
				[{ t: '  "next": ', c: 'k' }, { t: '"After you ship a fix, add_marker then verify_fix."', c: 's' }],
				[{ t: '}' }]
			]
		},
		{
			tool: 'verify_fix',
			ask: 'I shipped v1.4.3. Did the KeyError stop?',
			out: [
				[{ t: '{' }],
				[{ t: '  "pattern_id": ', c: 'k' }, { t: '"p_7c1e9a"', c: 'p' }, { t: ',' }],
				[{ t: '  "split": ', c: 'k' }, { t: '{ "marker": ' }, { t: '"v1.4.3"', c: 's' }, { t: ', "at": ' }, { t: '"09:47"', c: 's' }, { t: ' },' }],
				[{ t: '  "before": ', c: 'k' }, { t: '{ "count": ' }, { t: '388', c: 'n' }, { t: ', "per_hour": ' }, { t: '465.6', c: 'n' }, { t: ' },' }],
				[{ t: '  "after": ', c: 'k' }, { t: '{ "count": ' }, { t: '0', c: 'n' }, { t: ', "per_hour": ' }, { t: '0', c: 'n' }, { t: ' },' }],
				[{ t: '  "expected_after_if_unchanged": ', c: 'k' }, { t: '116.4', c: 'n' }, { t: ',' }],
				[{ t: '  "verdict": ', c: 'k' }, { t: '"fixed"', c: 'g' }, { t: ',' }],
				[{ t: '  "confidence": ', c: 'k' }, { t: '"high"', c: 'g' }, { t: ',' }],
				[{ t: '  "explanation": ', c: 'k' }, { t: '"~116 expected at the old rate, 0 seen."', c: 's' }],
				[{ t: '}' }]
			]
		},
		{
			tool: 'compare_deploy',
			ask: 'What did the last deploy break?',
			out: [
				[{ t: '{' }],
				[{ t: '  "split": ', c: 'k' }, { t: '{ "marker": ' }, { t: '"v1.4.2"', c: 's' }, { t: ', "at": ' }, { t: '"08:57"', c: 's' }, { t: ' },' }],
				[{ t: '  "summary": ', c: 'k' }, { t: '"2 new, 1 increased, 1 stopped"', c: 's' }, { t: ',' }],
				[{ t: '  "findings": [', c: 'k' }],
				[{ t: '    { ' }, { t: '"kind": ', c: 'k' }, { t: '"new"', c: 'b' }, { t: ', ' }, { t: '"template": ', c: 'k' }, { t: '"charge failed ... | KeyError"', c: 's' }, { t: ' },' }],
				[{ t: '    { ' }, { t: '"kind": ', c: 'k' }, { t: '"new"', c: 'b' }, { t: ', ' }, { t: '"template": ', c: 'k' }, { t: '"currency <str> not in rate table"', c: 's' }, { t: ' },' }],
				[{ t: '    { ' }, { t: '"kind": ', c: 'k' }, { t: '"increased"', c: 'm' }, { t: ', ' }, { t: '"template": ', c: 'k' }, { t: '"retrying payment provider ..."', c: 's' }, { t: ' },' }],
				[{ t: '    { ' }, { t: '"kind": ', c: 'k' }, { t: '"stopped"', c: 'g' }, { t: ', ' }, { t: '"template": ', c: 'k' }, { t: '"legacy tax path used for <*>"', c: 's' }, { t: ' }' }],
				[{ t: '  ]' }],
				[{ t: '}' }]
			]
		}
	];

	let active = $state(0);
	const demo = $derived(demos[active]);
</script>

<div class="term">
	<div class="bar">
		<span class="dots" aria-hidden="true"><i></i><i></i><i></i></span>
		<div class="tabs" role="tablist" aria-label="Example tool calls">
			{#each demos as d, i}
				<button
					role="tab"
					id="tab-{d.tool}"
					aria-selected={active === i}
					aria-controls="panel"
					class:on={active === i}
					onclick={() => (active = i)}>{d.tool}</button
				>
			{/each}
		</div>
	</div>
	<div class="body" id="panel" role="tabpanel" aria-labelledby="tab-{demo.tool}">
		<p class="ask"><span>agent</span> {demo.ask}</p>
		<p class="call">→ {demo.tool}()</p>
		{#key active}
			<pre class="out">{#each demo.out as line, i}<span class="ln" style="--d:{i * 35}ms">{#each line as seg}<span class={seg.c}>{seg.t}</span>{/each}</span>
{/each}</pre>
		{/key}
	</div>
</div>

<style>
	.term {
		border-radius: var(--radius);
		background: linear-gradient(180deg, rgb(19, 23, 50), rgb(10, 12, 28));
		border: 1px solid var(--line-strong);
		box-shadow:
			0 30px 80px -30px rgba(0, 0, 0, 0.8),
			0 0 0 1px rgba(255, 255, 255, 0.02) inset;
		overflow: hidden;
	}
	.bar {
		display: flex;
		align-items: center;
		gap: 16px;
		padding: 10px 14px;
		border-bottom: 1px solid var(--line);
		background: rgba(255, 255, 255, 0.02);
	}
	.dots {
		display: flex;
		gap: 6px;
		flex: none;
	}
	.dots i {
		width: 10px;
		height: 10px;
		border-radius: 50%;
		background: #2a3060;
	}
	.tabs {
		display: flex;
		gap: 4px;
		overflow-x: auto;
		scrollbar-width: none;
	}
	.tabs button {
		font: 500 0.8rem var(--font-mono);
		color: var(--muted);
		background: none;
		border: 1px solid transparent;
		padding: 6px 12px;
		border-radius: 8px;
		cursor: pointer;
		white-space: nowrap;
	}
	.tabs button:hover {
		color: var(--text);
	}
	.tabs button.on {
		color: var(--star);
		background: rgba(255, 200, 87, 0.08);
		border-color: rgba(255, 200, 87, 0.25);
	}
	.body {
		padding: 18px 20px 22px;
		font-family: var(--font-mono);
		font-size: 0.82rem;
		min-height: 420px;
	}
	.ask {
		margin: 0 0 4px;
		color: var(--text);
		font-family: var(--font-body);
		font-size: 0.95rem;
	}
	.ask span {
		font: 600 0.7rem var(--font-mono);
		text-transform: uppercase;
		letter-spacing: 0.1em;
		color: var(--violet);
		margin-right: 8px;
	}
	.call {
		margin: 0 0 12px;
		color: var(--sat);
	}
	.out {
		margin: 0;
		line-height: 1.65;
		color: #c6cbef;
		overflow-x: auto;
	}
	.ln {
		display: inline-block;
		animation: in 0.35s ease both;
		animation-delay: var(--d);
	}
	.k {
		color: #8fa8ff;
	}
	.s {
		color: #d6dbff;
	}
	.n {
		color: var(--star);
	}
	.g {
		color: var(--good);
	}
	.b {
		color: var(--bad);
	}
	.m {
		color: #ffb86b;
	}
	.p {
		color: var(--sat);
	}
	@keyframes in {
		from {
			opacity: 0;
			transform: translateY(4px);
		}
	}
	@media (max-width: 600px) {
		.body {
			font-size: 0.72rem;
			padding: 14px;
			min-height: 380px;
		}
	}
</style>
