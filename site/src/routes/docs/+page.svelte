<script lang="ts">
	import Code from '#lib/docs/Code.svelte';
	import ToolDoc from '#lib/docs/ToolDoc.svelte';
	import { tools, groups, json } from '#lib/docs/tools.ts';

	const repo = 'https://github.com/p-lau/starlist';
	const title = 'Docs · Starlist';
	const description =
		'How to run Starlist, what each MCP tool answers with real examples, and how to use it with Docker, Kubernetes, CI and more.';

	const groupOrder = Object.keys(groups) as (keyof typeof groups)[];

	const install = `# Any platform with Rust
cargo install starlist

# Or the container (Linux amd64 and arm64)
docker pull ghcr.io/p-lau/starlist:1`;

	const setup = `cd your-app
starlist init      # finds the logs, writes starlist.toml, sets up your agent
starlist serve     # reads starlist.toml; leave it running`;

	const byHand = `# Follow a file and serve MCP at http://127.0.0.1:7341/mcp
starlist serve --file /var/log/myapp.log

# Or pipe your app through it and still see your logs
npm start | starlist serve --stdin --tee

# Or let the agent launch it over stdio (reads the database a collector writes)
starlist collect --file app.log &
starlist mcp --stdio`;

	const connect = `claude mcp add --transport http logs http://127.0.0.1:7341/mcp`;

	const demo = `git clone https://github.com/p-lau/starlist && cd starlist
# 26 hours of a fake checkout service: a bad deploy 50m ago, a fix 15m ago
cargo run --release --example demo_log -- demo.log
cargo run --release -- serve --file demo.log --from-start --db demo.db
# Then open http://127.0.0.1:7341/ or point your agent at /mcp`;

	const templates = `ERROR charge failed for order 99812: "EURX"  ┐
ERROR charge failed for order 10442: "GBPX"  ┴─ ERROR charge failed for order <NUM>: "<STR>"
                                                pattern pb5d540dae8, 2 events`;

	const resource = {
		uri: 'starlist://whats-new',
		on: 'notifications/resources/updated',
		then: 'resources/read → whats_new for the last hour + latest_alert'
	};

	const webhook = `starlist serve --file app.log --notify-url https://hooks.example.com/starlist`;

	const api = `curl -s 'http://127.0.0.1:7341/api/v1/whats-new?window=1h&min_level=WARN'
curl -s http://127.0.0.1:7341/api/v1/patterns/p6c8d499767/verify?since=v1.4.3
# Beyond localhost, every request needs the token
curl -s -H "Authorization: Bearer $(cat starlist.db.token)" http://host:7341/api/v1/status`;

	const history = `# Keep p50/p95/p99 of a field per pattern, hourly, for months
starlist serve --file app.log --metric duration_ms

# Serve /metrics for Prometheus
starlist serve --file app.log --prometheus

# Patterns, hourly counts, percentiles and markers as NDJSON
starlist export --since 30d --db starlist.db > starlist.ndjson`;

	const hub = `# On the hub host: one push token per service, then run the hub
starlist hub-token api --dir /var/lib/starlist-hub
starlist hub-token pricing --dir /var/lib/starlist-hub
starlist hub --dir /var/lib/starlist-hub --http 0.0.0.0:7341

# Next to each app (replicas share one service name)
STARLIST_HUB_TOKEN=... starlist serve --file app.log \\
  --hub https://hub.internal:7341 --service api --env production`;

	const ci = `# Save each run's output
cargo test 2>&1 | tee test.log

# After a failure, against the last passing run's log
starlist-ci --baseline passing.log failing.log
starlist-ci --baseline passing.log --format markdown < failing.log

# No passing log yet: list the failing log's errors and warnings
starlist-ci failing.log`;

	const ciInstall = `cargo install --git https://github.com/p-lau/starlist starlist-ci`;

	const ciAnswer = {
		summary:
			'3 new, 1 grew, 2 stopped compared with the passing run. Start with line 8: ERROR payment failed for order <NUM> | ValueError',
		events: { failing: 9, passing: 6 },
		findings: [
			{
				kind: 'new',
				level: 'ERROR',
				template: 'ERROR payment failed for order <NUM> | ValueError',
				count: 1,
				before: 0,
				location: 'billing/charge.py:88 in charge',
				step: 'Run tests',
				line: 8
			},
			{ kind: 'grew', level: 'WARN', template: 'WARN retrying request <NUM> of <NUM>', count: 3, before: 1, line: 4 },
			{ kind: 'stopped', level: 'INFO', template: 'INFO seeding fixtures done', count: 0, before: 1, passing_line: 6 }
		]
	};

	const toml = `# starlist.toml: each key is a command-line flag without the dashes
file = ["logs/app.log"]
docker = ["worker"]
db = ".starlist/starlist.db"
source-root = ["/app"]
redact = ['order_secret=\\w+']
metric = ["duration_ms"]
env = "production"
tools = "core"`;

	const flags: [string, string][] = [
		['--file PATH', 'Follow a file like tail -F (rotation-safe). Repeatable.'],
		['--stdin, --tee', 'Read a pipe; --tee also prints the lines so you still see them.'],
		['--docker, --journald, --k8s, --otlp, --loki', 'Other log sources; see Integrations.'],
		['--from-start', 'Read what a source already has, not only new lines.'],
		['--http ADDR', 'Where to listen. Default 127.0.0.1:7341.'],
		['--token TOKEN', 'Bearer token (STARLIST_TOKEN). Required beyond localhost; generated if missing.'],
		['--db PATH', 'The SQLite file. Default starlist.db.'],
		['--tools core', 'Offer only diagnose, whats_new, get_pattern, get_context, get_request and verify_fix, or any list.'],
		['--source-root DIR', 'Strip a path prefix (like /app) so code locations match your repository.'],
		['--redact REGEX', 'Redact more than the built-in rules. Repeatable.'],
		['--request-key KEY', 'Another field that holds a request ID.'],
		['--group REGEX', 'One pattern per (?P<name>…) value for lines masking splits apart.'],
		['--merge-after N', 'Merge patterns that differ in one word after N values. Default 5; 0 turns it off.'],
		['--metric FIELD', 'Keep hourly percentiles of a numeric field per pattern.'],
		['--log-tz +02:00', 'Zone for timestamps that have none. Default UTC.'],
		['--raw-size 20MB, --no-raw', 'Disk for recent redacted lines (get_context, get_request), or none.'],
		['--retention 7d, --history 400d', 'How long fine-grained and hourly counts are kept.'],
		['--env NAME', 'Label for what this collector watches (STARLIST_ENV).'],
		['--notify-url URL, --no-alerts', 'Alert webhook, or no alerts at all.'],
		['--no-dashboard, --no-api, --prometheus', 'Turn the dashboard or JSON API off, or /metrics on.']
	];
</script>

<svelte:head>
	<title>{title}</title>
	<meta name="description" content={description} />
	<meta property="og:title" content={title} />
	<meta property="og:description" content={description} />
</svelte:head>

<article class="doc">
	<header>
		<p class="eyebrow">Docs · Guide</p>
		<h1>Put a satellite in orbit.</h1>
		<p class="lede">
			Starlist runs next to your app, reads its logs and answers your AI agent over MCP. This guide
			covers setting it up, every tool the agent gets with a real call and answer, and the features
			around them. For Docker, Kubernetes, CI and other platforms, see
			<a href="/docs/integrations/">Integrations</a>.
		</p>
	</header>

	<ul class="cards">
		<li><a href="#start"><b>Get started</b>Install, run <code>starlist init</code> and connect your agent.</a></li>
		<li><a href="#tools"><b>Tools</b>What each MCP tool answers, with a real example.</a></li>
		<li><a href="/docs/integrations/"><b>Integrations</b>Docker, Kubernetes, journald, CI/CD, Cloudflare.</a></li>
		<li><a href="#ci"><b>starlist-ci</b>Why did this CI run fail, compared with the last green one.</a></li>
	</ul>

	<!-- START -->
	<section id="start">
		<p class="eyebrow">Launch</p>
		<h2>Get started</h2>
		<p>
			Starlist is one small binary (about 5 MB, about 11 MB of memory while serving). It works with
			any language, because it only reads the text your app already writes: plain lines or JSON,
			including Python, Java, Node, Go and Rust stack traces.
		</p>

		<h3>1. Install</h3>
		<Code title="install" code={install} />
		<p>
			Prebuilt binaries for Linux (x86_64 and arm64, static), macOS and Windows are on the
			<a href="{repo}/releases/latest" rel="noopener">releases page</a>, with <code>SHA256SUMS</code>.
		</p>

		<h3>2. Point it at your logs</h3>
		<Code title="in your project" code={setup} />
		<p>
			<code>starlist init</code> looks for log files in the project (and <code>/var/log/&lt;project&gt;</code>),
			a Docker Compose project or containers named after it, systemd units, Kubernetes workloads in
			your manifests, and an OpenTelemetry SDK in your dependencies. It shows what it found, lets you
			pick, and writes <code>starlist.toml</code> plus your agent's MCP settings. It asks before
			changing a file that exists; <code>--dry-run</code> prints the files instead.
		</p>
		<p>Prefer flags? Everything <code>init</code> writes is also a command-line option:</p>
		<Code title="run it by hand" code={byHand} />

		<h3>3. Connect your agent</h3>
		<p>
			While <code>starlist serve</code> runs, the MCP endpoint is <code>http://127.0.0.1:7341/mcp</code>
			(streamable HTTP). <code>init</code> already set up Claude Code, Cursor, VS Code or Gemini CLI if
			the project uses them. By hand, for Claude Code:
		</p>
		<Code title="Claude Code" code={connect} />
		<p>Other clients are in <a href="/docs/integrations/#agents">MCP clients</a>.</p>

		<h3>Try it on the demo</h3>
		<p>No app handy? The repository ships a demo log with a bad deploy and its fix. Every example on this page came from it.</p>
		<Code title="demo" code={demo} />
	</section>

	<!-- CONCEPTS -->
	<section id="concepts">
		<p class="eyebrow">Orbital mechanics</p>
		<h2>How starlist reads logs</h2>
		<p>Every line goes through the same steps, whatever source it came from:</p>
		<ol>
			<li><strong>Collect.</strong> Stack-trace lines are kept with the log line that produced them, so one exception is one event.</li>
			<li><strong>Redact</strong> before anything is stored: emails, IPs, bearer tokens, JWTs, API keys (OpenAI, GitHub, Slack, Stripe, AWS), <code>password=</code> values, card numbers, private keys and your own <code>--redact</code> rules.</li>
			<li><strong>Pattern.</strong> Numbers, IDs, UUIDs, hex, quoted strings and timestamps are masked. The exception type is part of the pattern.</li>
			<li><strong>Store</strong> in a local SQLite file: per-10-second counts for 7 days, hourly counts for 400 days, 5 recent samples per pattern, and markers.</li>
		</ol>
		<Code title="many lines, one pattern" lang="text" code={templates} />

		<h3>Pattern IDs are stable</h3>
		<p>
			A pattern's ID is a hash of its template, so it is the same after a restart, on another host and
			on a hub. An agent can note an ID before a fix and ask about it after. When patterns differ in
			just one word that has taken 5 different values (a username, a hostname), they are merged into
			one with <code>&lt;*&gt;</code> there, and the merged IDs keep resolving to it.
		</p>

		<h3>Markers and releases</h3>
		<p>
			A marker names a point in time. Deploys are detected from lines like
			<code>Starting api version 1.4.2</code>, from <code>STARLIST_VERSION</code> at startup, from
			<code>starlist mark v1.4.3 --deploy</code> or from the <code>add_marker</code> tool. Every pattern
			carries <code>first_release</code>, the release that was live when it first appeared.
		</p>

		<h3>Times</h3>
		<p>Anywhere a tool takes a time, it accepts <code>30m</code>, <code>2h</code>, an ISO time or a marker name like <code>v1.4.3</code>.</p>

		<h3>Answers are sized for agent context</h3>
		<ul>
			<li><strong>Brief by default:</strong> compact JSON, relative times (<code>21m ago</code>), samples only on the top findings, stack traces trimmed to your app's own frames. Pass <code>detail: "full"</code> for absolute times, raw rates and untrimmed samples.</li>
			<li><strong>A hard size limit:</strong> 8,000 characters brief (about 2k tokens) and 16,000 full, or <code>max_chars</code> (at least 1,000). Samples are shortened, then dropped, then findings move to the next page, and <code>trimmed</code> says what was cut.</li>
			<li><strong>Paging:</strong> <code>whats_new</code>, <code>compare_deploy</code> and <code>list_patterns</code> return <code>next_cursor</code>; pass it back as <code>cursor</code>.</li>
			<li><strong>Fewer definitions:</strong> tool definitions are sent with every model call. <code>--tools core</code> offers six tools, about half the size; <code>--tools diagnose,get_pattern</code> picks any set.</li>
		</ul>
	</section>

	<!-- TOOLS -->
	<section id="tools">
		<p class="eyebrow">The tool belt</p>
		<h2>Tools</h2>
		<p>
			Fourteen MCP tools, grouped by the question they answer. Each example is a real call and answer
			from the demo, pretty-printed, with long stack traces cut. Every tool also takes
			<code>detail</code> and <code>max_chars</code>. The exact argument and field lists are in
			<a href="{repo}/blob/main/docs/tools.md" rel="noopener">docs/tools.md</a>.
		</p>
		<div class="table">
			<table>
				<thead><tr><th>Question</th><th>Tools</th></tr></thead>
				<tbody>
					{#each groupOrder as g}
						<tr>
							<td>{groups[g]}</td>
							<td>
								{#each tools.filter((t) => t.group === g) as t, i}{#if i},{' '}{/if}<a href="#{t.name}"><code>{t.name}</code></a>{/each}
							</td>
						</tr>
					{/each}
				</tbody>
			</table>
		</div>

		{#each groupOrder as g}
			<h3 id="tools-{g}">{groups[g]}</h3>
			{#each tools.filter((t) => t.group === g) as tool}
				<ToolDoc {tool} />
			{/each}
		{/each}

		<h3 id="resource">The whats_new resource</h3>
		<p>
			Besides tools, starlist offers one MCP resource. A client that subscribes to it is told within
			seconds when a new, returning or spiking ERROR or FATAL pattern appears, then reads it. See
			<a href="#alerts">Alerts</a>.
		</p>
		<Code lang="json" title="resource" code={json(resource)} />

		<div class="note">
			<p>
				<strong>When to reach for starlist.</strong> If a stack trace or a clear symptom already points
				at recent code, reading the code is usually cheaper. Starlist pays off when the symptom is
				unknown, nothing was deployed, the codebase is large, or the cause may be outside the code (an
				upstream service, data, config, clients). The server's MCP instructions tell agents the same.
			</p>
		</div>
	</section>

	<!-- WALKTHROUGH -->
	<section id="walkthrough">
		<p class="eyebrow">A full orbit</p>
		<h2>Walkthrough: a bad deploy</h2>
		<p>
			Here is how an agent uses the tools together on the demo, where release v1.4.2 broke checkout.
			You ask: <em>"Checkout is failing for some users. What's wrong?"</em>
		</p>
		<ol class="walk">
			<li>
				<strong><code>diagnose</code></strong> comes back with one likely cause: a new
				<code>KeyError</code> at <code>/app/pricing.py:41 in price_cart</code>, which started with
				<code>v1.4.2</code>, 27 times in the last hour. It also notes a new payments timeout.
			</li>
			<li>
				<strong><code>get_pattern</code></strong> shows the histogram: zero for 23 hours, then 27 right
				after the deploy. The sample's stack trace reads <code>currency = cart.meta['currency']</code>.
			</li>
			<li>
				The agent reads <code>pricing.py</code>, sees carts without a currency, and ships a fix as
				<code>v1.4.3</code>. Its startup line records the deploy (or the agent calls
				<code>add_marker</code>).
			</li>
			<li>
				<strong><code>verify_fix</code></strong> with <code>since: "v1.4.3"</code> says
				<code>fixed</code>: about 12 would have occurred at the old rate, and none did. It also warns
				that traffic dropped, so the confidence is worth a second look later.
			</li>
			<li>
				<strong><code>set_pattern_state</code></strong> marks it <code>resolved</code>. If it ever
				comes back, <code>whats_new</code> and <code>get_release</code> report it as
				<code>regressed</code>.
			</li>
		</ol>
		<p>The whole investigation costs a few thousand tokens, instead of reading a 26-hour log.</p>
	</section>

	<!-- ALERTS -->
	<section id="alerts">
		<p class="eyebrow">Signals</p>
		<h2>Alerts</h2>
		<p>Agents don't have to poll. There are two ways to hear about a new problem:</p>
		<ul>
			<li>
				<strong>MCP subscription.</strong> A client that subscribes to <code>starlist://whats-new</code>
				gets <code>notifications/resources/updated</code> within seconds of a new, returning or spiking
				ERROR/FATAL pattern. One incident is one notification: patterns found together are gathered
				for 10 seconds, alerts are at least a minute apart, and a pattern isn't alerted again for 30
				minutes.
			</li>
			<li>
				<strong>Webhook.</strong> Without a connected agent, <code>--notify-url</code> (or
				<code>STARLIST_NOTIFY_URL</code>) POSTs each alert as JSON with <code>summary</code> and
				<code>findings</code>. It's starlist's own JSON, so for a chat tool, put a small relay in
				between that turns it into that tool's message format.
			</li>
		</ul>
		<Code title="webhook" code={webhook} />
		<p>
			A pattern that is busy at set times, like a nightly job, is labelled <code>usual</code> and not
			alerted unless it's worse than earlier days. <code>set_pattern_state</code> with
			<code>timing</code> overrides that either way. <code>--no-alerts</code> turns alerts off.
		</p>
	</section>

	<!-- DASHBOARD -->
	<section id="dashboard">
		<p class="eyebrow">Mission control</p>
		<h2>Dashboard and JSON API</h2>
		<p>
			<code>starlist serve</code> also serves a dashboard at <code>http://127.0.0.1:7341/</code>: log
			volume by level with deploys marked, what's new, a searchable pattern list with trends, each
			pattern's samples, locations, surrounding lines and requests, a "did the fix work?" check, and
			what changed around each deploy. It's about 50 KB, loads nothing from elsewhere, and uses the
			same token as MCP. <code>--no-dashboard</code> turns it off.
		</p>
		<p>
			The same process serves a JSON API at <code>/api/v1</code> for scripts and your own UIs. It
			returns data rather than agent summaries: ISO times, numbers, full samples. Routes include
			<code>status</code>, <code>timeline</code>, <code>diagnose</code>, <code>whats-new</code>,
			<code>patterns</code>, <code>verify</code>, <code>compare</code>, <code>context</code>,
			<code>requests</code>, <code>releases</code>, <code>fields</code>, <code>markers</code> and a
			server-sent event stream of alerts. See
			<a href="{repo}/blob/main/docs/api.md" rel="noopener">docs/api.md</a>.
		</p>
		<Code title="api" code={api} />
		<p>Calling it from a page on another origin needs <code>--cors-origin http://localhost:5173</code>.</p>
	</section>

	<!-- HISTORY -->
	<section id="history">
		<p class="eyebrow">Long exposure</p>
		<h2>History, metrics and export</h2>
		<p>
			Fine-grained counts are kept for 7 days (<code>--retention</code>), then folded into hourly
			counts kept for 400 days (<code>--history</code>), so a month-over-month question still has an
			answer. A pattern costs about 45 bytes for each hour it occurs in.
		</p>
		<ul>
			<li><code>--metric duration_ms</code> reads that field from each line (<code>duration_ms=532</code>, <code>latency: 0.53</code> or JSON) and keeps an hourly histogram per pattern, so p50, p95 and p99 survive for months without the lines.</li>
			<li><code>--prometheus</code> serves <code>/metrics</code>: events per level, patterns per level and state, each metric's recent percentiles, and the database size.</li>
			<li><code>starlist export</code> writes patterns, hourly counts, percentiles and markers as one JSON object per line, for a warehouse, a spreadsheet or a backup.</li>
		</ul>
		<Code title="history" code={history} />
	</section>

	<!-- HUB -->
	<section id="hub">
		<p class="eyebrow">Ground station</p>
		<h2>Hub for many services</h2>
		<p>
			For more than one app, run a hub and point collectors at it. Every collector keeps working on its
			own (its own MCP, API and dashboard) and pushes a copy of what it records. The hub is optional;
			one collector per app is the usual setup.
		</p>
		<Code title="hub" code={hub} />
		<ul>
			<li><strong>What is pushed:</strong> events (already redacted and fingerprinted), markers and merges. Nothing is pulled from collectors, and a push token can only write its own service.</li>
			<li><strong>Replicas</strong> of a service add up in one place, because pattern IDs are the same on every host. A deploy reported by every replica is recorded once.</li>
			<li><strong>Outages:</strong> a collector queues batches in its own database (up to <code>--hub-buffer</code>, 200 MB by default) and catches up afterwards. A retried batch is applied once.</li>
			<li><strong>Asking the hub:</strong> the same tools with a <code>service</code> argument, plus <code>list_services</code>. <code>whats_new</code> and <code>diagnose</code> without a service cover them all, and <code>get_request</code> without one follows a request across services.</li>
			<li>The hub has no TLS of its own, so put it behind an HTTPS proxy. It has no push alerts or <code>/metrics</code> of its own yet; each collector still has both.</li>
		</ul>
	</section>

	<!-- CI -->
	<section id="ci">
		<p class="eyebrow">Launch telemetry</p>
		<h2>starlist-ci for CI logs <span class="kind soon">next release</span></h2>
		<p>
			A CI job starts fresh, so there is no history to compare with. <code>starlist-ci</code> is a
			separate small binary that compares a failing run's log with the last passing run's and reports
			what is new, what grew and what stopped, with line numbers, steps and code locations. It prints
			JSON for agents or Markdown for people, and writes nothing to disk.
		</p>
		<Code title="starlist-ci" code={ci} />
		<Code lang="json" title="answer (trimmed)" code={json(ciAnswer)} wrap />
		<div class="note warn">
			<p>
				<code>starlist-ci</code> is merged but not in a release yet, so it isn't on crates.io or the
				releases page. Until it is, build it from the repository:
			</p>
			<Code title="install from source" code={ciInstall} />
		</div>
		<p>
			It reads plain output, <code>gh run view --log</code> output and raw GitHub Actions logs. Ready-made
			setups are in <a href="/docs/integrations/#github-actions">GitHub Actions</a>,
			<a href="/docs/integrations/#gitlab">GitLab CI</a> and <a href="/docs/integrations/#jenkins">Jenkins</a>.
		</p>
	</section>

	<!-- CONFIG -->
	<section id="config">
		<p class="eyebrow">Flight plan</p>
		<h2>Configuration</h2>
		<p>
			<code>starlist.toml</code> holds command-line flags: each key is a flag without the dashes.
			<code>serve</code>, <code>collect</code>, <code>mcp</code>, <code>ingest</code>, <code>export</code>
			and <code>mark</code> read it from the current directory, or from <code>--config PATH</code>
			(<code>STARLIST_CONFIG</code>). Flags on the command line win, lists add to the file's, and
			relative paths are read from the file's directory.
		</p>
		<Code lang="toml" title="starlist.toml" code={toml} />
		<h3>Common flags</h3>
		<div class="table">
			<table>
				<thead><tr><th>Flag</th><th>What it does</th></tr></thead>
				<tbody>
					{#each flags as [f, d]}
						<tr><td><code>{f}</code></td><td>{d}</td></tr>
					{/each}
				</tbody>
			</table>
		</div>
		<p>
			The commands: <code>init</code>, <code>serve</code> (collect and serve, the usual way),
			<code>collect</code> and <code>mcp</code> (the two halves on their own), <code>ingest</code>
			(import old files and exit), <code>export</code>, <code>mark</code>, <code>hub</code> and
			<code>hub-token</code>. <code>starlist &lt;command&gt; --help</code> lists every flag.
		</p>
	</section>

	<!-- SECURITY -->
	<section id="security">
		<p class="eyebrow">Shielding</p>
		<h2>Security</h2>
		<ul>
			<li>Redaction happens before anything is stored, so what it recognizes never reaches the database, the tools, the API or a hub.</li>
			<li>On localhost (the default) no token is needed, and only loopback <code>Host</code> headers are accepted, which blocks DNS-rebinding tricks from a browser.</li>
			<li>Listening beyond localhost (<code>--http 0.0.0.0:7341</code>) requires a bearer token: <code>--token</code>, <code>STARLIST_TOKEN</code>, or one generated next to the database as <code>&lt;db&gt;.token</code>.</li>
			<li>Starlist has no TLS of its own. To reach it from another machine, put an HTTPS proxy or a tunnel in front; see <a href="/docs/integrations/#remote">Reaching it remotely</a>.</li>
			<li>To answer <code>get_context</code> and <code>get_request</code>, the most recent redacted lines are kept, up to <code>--raw-size</code>. If that's a concern, add <code>--redact</code> rules or run with <code>--no-raw</code>.</li>
		</ul>
		<p>To report a vulnerability, see <a href="{repo}/blob/main/SECURITY.md" rel="noopener">SECURITY.md</a>.</p>
	</section>
</article>

<style>
	.walk li {
		margin-bottom: 10px;
	}
</style>
