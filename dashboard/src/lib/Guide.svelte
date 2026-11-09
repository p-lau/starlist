<script lang="ts">
	import { closeGuide } from './guide.svelte.ts';
</script>

<section class="card guide" aria-labelledby="guide-title">
	<div class="head">
		<h2 id="guide-title">How to read this dashboard</h2>
		<button onclick={closeGuide}>Got it</button>
	</div>
	<ol>
		<li>
			<b>Lines become patterns.</b> starlist turns each log line into a template by swapping the parts that change,
			like ids and numbers, for placeholders. Lines with the same template count as one pattern.
		</li>
		<li>
			<b>Start with What's new.</b> On the <a href="/">Overview</a> it lists the patterns that are new, back, or
			busier or quieter than the day before. That is usually where a problem shows first.
		</li>
		<li>
			<b>Open a pattern to dig in.</b> Click any template to see when it started, sample lines, the numbers in
			them, and a request it came from. Browse them all under <a href="/patterns">Patterns</a>.
		</li>
		<li>
			<b>Triage it.</b> Resolve a pattern once it is fixed, and starlist marks it regressed if it comes back. Ignore
			one to keep it out of What's new and alerts.
		</li>
		<li>
			<b>Check a fix against a deploy.</b> <a href="/deploys">Deploys</a> shows what changed with each release, and
			“Did a fix work?” on a pattern compares its rate before and after one.
		</li>
	</ol>
	<p class="faint small">Look for the <span class="q">?</span> marks for short explanations. Reopen this guide from the header.</p>
</section>

<style>
	.guide {
		margin-bottom: 16px;
		border-color: var(--accent);
	}
	.head {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 12px;
	}
	h2 {
		margin: 0;
	}
	ol {
		margin: 12px 0 8px;
		padding-left: 1.3em;
		display: grid;
		gap: 6px;
	}
	.small {
		font-size: 0.8rem;
		margin: 0;
	}
	.q {
		display: inline-grid;
		place-items: center;
		width: 16px;
		height: 16px;
		border-radius: 50%;
		border: 1px solid var(--line-strong);
		font-size: 0.65rem;
		font-weight: 700;
	}
</style>
