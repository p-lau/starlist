<script lang="ts">
	import Code from '#lib/docs/Code.svelte';

	const repo = 'https://github.com/p-lau/starlist';
	const title = 'Integrations · Starlist docs';
	const description =
		'Run Starlist with Docker, Kubernetes, journald, OpenTelemetry and Loki, compare failing CI runs in GitHub Actions, GitLab CI and Jenkins, and record deploys from any pipeline.';

	// MCP clients
	const claude = `claude mcp add --transport http starlist http://127.0.0.1:7341/mcp

# Beyond localhost, with the token
claude mcp add --transport http starlist https://logs.example.com/mcp \\
  --header "Authorization: Bearer $STARLIST_TOKEN"`;
	const cursor = `{ "mcpServers": { "starlist": { "url": "http://127.0.0.1:7341/mcp" } } }`;
	const vscode = `{ "servers": { "starlist": { "type": "http", "url": "http://127.0.0.1:7341/mcp" } } }`;
	const gemini = `{ "mcpServers": { "starlist": { "httpUrl": "http://127.0.0.1:7341/mcp" } } }`;
	const windsurf = `{ "mcpServers": { "starlist": { "serverUrl": "http://127.0.0.1:7341/mcp" } } }`;
	const codex = `codex mcp add starlist --url http://127.0.0.1:7341/mcp`;
	const stdio = `# The agent starts starlist itself; a collector fills the same database
starlist collect --file app.log --db .starlist/starlist.db &
starlist mcp --stdio --db .starlist/starlist.db`;

	// Docker
	const dockerCli = `# One container, by name or ID. Restarts and re-creation are followed.
starlist serve --docker shop-api

# Every container with a label, including ones started later
starlist serve --docker-label com.docker.compose.service=api

# Podman
STARLIST_DOCKER=podman starlist serve --docker shop-api`;

	const dockerRun = `docker run --rm -v app-logs:/logs:ro -v starlist:/data \\
  -p 127.0.0.1:7341:7341 -e STARLIST_TOKEN=$(openssl rand -hex 32) \\
  ghcr.io/p-lau/starlist:1 \\
  serve --file /logs/app.log --http 0.0.0.0:7341 --db /data/starlist.db`;

	const compose = `services:
  app:
    image: your-app
    # The app writes its log to /var/log/app/app.log
    volumes:
      - app-logs:/var/log/app

  starlist:
    image: ghcr.io/p-lau/starlist:1
    command:
      - serve
      - --file=/var/log/app/app.log
      - --http=0.0.0.0:7341
      - --db=/data/starlist.db
      - --env=production
    environment:
      STARLIST_TOKEN: \${STARLIST_TOKEN:?set STARLIST_TOKEN}
      # STARLIST_VERSION: v1.4.3   # the app's release, if its logs don't say
    volumes:
      - app-logs:/var/log/app:ro
      - starlist-data:/data
    ports:
      - 127.0.0.1:7341:7341   # this machine only
    restart: unless-stopped

volumes:
  app-logs:
  starlist-data:`;

	const dockerSock = `# Dockerfile: starlist plus the Docker CLI, to read other containers' stdout
FROM docker:27-cli
COPY --from=ghcr.io/p-lau/starlist:1 /usr/local/bin/starlist /usr/local/bin/starlist
ENTRYPOINT ["starlist"]`;

	const dockerSockCompose = `  starlist:
    build: ./starlist
    command: serve --docker-label app=shop --http 0.0.0.0:7341 --db /data/starlist.db
    environment:
      STARLIST_TOKEN: \${STARLIST_TOKEN:?set STARLIST_TOKEN}
    volumes:
      - /var/run/docker.sock:/var/run/docker.sock:ro
      - starlist-data:/data`;

	// Kubernetes
	const k8sCli = `# Every pod of a deployment (also statefulset/, daemonset/, replicaset/, job/)
starlist serve --k8s deployment/api --k8s-namespace shop

# Or any label selector
starlist serve --k8s 'app=api,tier=web'`;

	const sidecar = `apiVersion: apps/v1
kind: Deployment
metadata:
  name: api
spec:
  selector:
    matchLabels: { app: api }
  template:
    metadata:
      labels: { app: api }
    spec:
      containers:
        - name: api
          image: your-app
          # The app writes its log to /var/log/app/app.log
          volumeMounts:
            - { name: app-logs, mountPath: /var/log/app }

        - name: starlist
          image: ghcr.io/p-lau/starlist:1
          args: [serve, --file=/var/log/app/app.log, --http=0.0.0.0:7341, --db=/data/starlist.db]
          env:
            - name: STARLIST_TOKEN
              valueFrom: { secretKeyRef: { name: starlist, key: token } }
            - name: STARLIST_ENV
              value: production
          ports:
            - { name: mcp, containerPort: 7341 }
          volumeMounts:
            - { name: app-logs, mountPath: /var/log/app, readOnly: true }
            - { name: starlist-data, mountPath: /data }
          resources:
            requests: { cpu: 10m, memory: 32Mi }
            limits: { memory: 128Mi }
          securityContext:
            runAsNonRoot: true
            readOnlyRootFilesystem: true
            allowPrivilegeEscalation: false
            capabilities: { drop: [ALL] }
      volumes:
        - name: app-logs
          emptyDir: {}
        - name: starlist-data
          emptyDir: { sizeLimit: 512Mi }   # a PersistentVolumeClaim keeps history`;

	const sidecarRun = `kubectl create secret generic starlist --from-literal=token=$(openssl rand -hex 32)
kubectl apply -f deploy/kubernetes/sidecar.yaml
kubectl port-forward deploy/api 7341
claude mcp add --transport http starlist http://127.0.0.1:7341/mcp \\
  --header "Authorization: Bearer $(kubectl get secret starlist -o jsonpath='{.data.token}' | base64 -d)"`;

	const rbac = `apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata: { name: starlist, namespace: shop }
rules:
  - apiGroups: [""]
    resources: [pods, pods/log]
    verbs: [get, list]
  - apiGroups: [apps]
    resources: [deployments, statefulsets, daemonsets, replicasets]
    verbs: [get]`;

	const kubectlImage = `# Dockerfile: starlist plus kubectl (pick your cluster's version and arch)
FROM alpine:3.20
ARG KUBECTL=v1.31.0
RUN wget -qO /usr/local/bin/kubectl \\
      https://dl.k8s.io/release/$KUBECTL/bin/linux/amd64/kubectl \\
 && chmod +x /usr/local/bin/kubectl
COPY --from=ghcr.io/p-lau/starlist:1 /usr/local/bin/starlist /usr/local/bin/starlist
ENTRYPOINT ["starlist"]`;

	// systemd
	const journald = `starlist serve --journald api.service --journald worker.service`;
	const unit = `# /etc/systemd/system/starlist.service
[Unit]
Description=starlist log collector
After=network.target

[Service]
User=starlist
Group=systemd-journal
WorkingDirectory=/var/lib/starlist
ExecStart=/usr/local/bin/starlist serve --journald api.service --db /var/lib/starlist/starlist.db
Restart=on-failure

[Install]
WantedBy=multi-user.target`;
	const unitRun = `sudo useradd --system --home /var/lib/starlist --create-home starlist
sudo systemctl daemon-reload && sudo systemctl enable --now starlist`;

	// OpenTelemetry
	const otlp = `starlist serve --otlp 127.0.0.1:4318

# Point an SDK's log exporter at it
OTEL_LOGS_EXPORTER=otlp \\
OTEL_EXPORTER_OTLP_LOGS_PROTOCOL=http/protobuf \\
OTEL_EXPORTER_OTLP_LOGS_ENDPOINT=http://127.0.0.1:4318/v1/logs \\
  python -m myapp`;
	const otelCollector = `exporters:
  otlphttp/starlist:
    logs_endpoint: http://starlist:4318/v1/logs
    headers: { Authorization: "Bearer \${env:STARLIST_OTLP_TOKEN}" }
service:
  pipelines:
    logs:
      exporters: [otlphttp/starlist]`;

	// Loki
	const loki = `starlist serve --loki http://loki:3100 --loki-query '{app="api", env="prod"}'

# Grafana Cloud: user ID and an access token, as basic auth
starlist serve --loki https://logs-prod-012.grafana.net --loki-user 123456 \\
  --loki-token "$GRAFANA_TOKEN" --loki-query '{service_name="api"}'`;

	// GitHub Actions
	const gha = `permissions:
  contents: read
  actions: read          # find the last passing run and download its log
  pull-requests: write   # only with comment: true

jobs:
  test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Tests
        shell: bash      # with pipefail, so tee keeps the test's exit code
        run: cargo test 2>&1 | tee test.log
      - uses: p-lau/starlist/ci@v1.1.0   # the first release with starlist-ci
        if: always()
        with:
          log: test.log
          status: \${{ job.status }}
          comment: true`;
	const ghaNow = `      # Until the release: put starlist-ci on PATH first, then use the action from main
      - run: cargo install --git https://github.com/p-lau/starlist starlist-ci
        if: failure()
      - uses: p-lau/starlist/ci@main
        if: always()
        with:
          log: test.log
          status: \${{ job.status }}`;
	const ghaAgent = `gh run view <failed-run-id> --log > failing.log
gh run view <last-passing-run-id> --log > passing.log
starlist-ci --baseline passing.log failing.log`;

	// GitLab
	const gitlab = `test:
  image: rust:1
  before_script:
    - cargo install --git https://github.com/p-lau/starlist starlist-ci
  script:
    - set -o pipefail
    - cargo test 2>&1 | tee test.log
  after_script:
    - |
      if [ "$CI_JOB_STATUS" = failed ]; then
        # The same job's log from the latest successful pipeline on the default branch
        curl -sf -H "PRIVATE-TOKEN: $STARLIST_READ_TOKEN" -o passing.log \\
          "$CI_API_V4_URL/projects/$CI_PROJECT_ID/jobs/artifacts/$CI_DEFAULT_BRANCH/raw/test.log?job=$CI_JOB_NAME" \\
          && starlist-ci --baseline passing.log --format markdown test.log > starlist-ci.md \\
          || starlist-ci --format markdown test.log > starlist-ci.md
        cat starlist-ci.md
      fi
  artifacts:
    when: always
    paths: [test.log, starlist-ci.md]
    expire_in: 14 days`;

	// Jenkins
	const jenkins = `pipeline {
  agent any
  stages {
    stage('Test') {
      steps {
        sh '''#!/bin/bash
          set -o pipefail
          cargo test 2>&1 | tee test.log
        '''
      }
    }
  }
  post {
    always {
      archiveArtifacts artifacts: 'test.log', allowEmptyArchive: true
    }
    failure {
      // Copy Artifact plugin: this job's log from its last successful build
      copyArtifacts projectName: env.JOB_NAME, selector: lastSuccessful(),
                    filter: 'test.log', target: 'baseline', optional: true
      sh '''
        if [ -f baseline/test.log ]; then
          starlist-ci --baseline baseline/test.log --format markdown test.log > starlist-ci.md
        else
          starlist-ci --format markdown test.log > starlist-ci.md
        fi
        cat starlist-ci.md
      '''
      archiveArtifacts artifacts: 'starlist-ci.md'
    }
  }
}`;
	const jenkinsCurl = `curl -sf -u "$JENKINS_USER:$JENKINS_API_TOKEN" -o passing.log \\
  "\${JOB_URL}lastSuccessfulBuild/artifact/test.log"`;

	// Other CI
	const genericCi = `# 1. Every run: keep the job's output as an artifact
make test 2>&1 | tee test.log

# 2. On failure: fetch test.log from the last green run (your CI's API or CLI),
#    then compare and post the Markdown wherever your CI shows reports
starlist-ci --baseline passing.log --format markdown test.log > report.md`;

	// Deploys
	const markCurl = `curl -sf -X POST https://logs.example.com/api/v1/markers \\
  -H "Authorization: Bearer $STARLIST_TOKEN" \\
  -H "Content-Type: application/json" \\
  -d '{"name": "v1.4.3", "deploy": true, "note": "deployed by CI"}'`;
	const markAnswer = `HTTP/1.1 201 Created
{"name":"v1.4.3","kind":"deploy","at":"2026-11-12T04:36:02.727Z","note":"deployed by CI"}`;
	const markGha = `      - name: Tell starlist about the release
        if: success()
        run: |
          curl -sf -X POST "$STARLIST_URL/api/v1/markers" \\
            -H "Authorization: Bearer $STARLIST_TOKEN" -H "Content-Type: application/json" \\
            -d "{\\"name\\": \\"$GITHUB_REF_NAME\\", \\"deploy\\": true, \\"note\\": \\"$GITHUB_SHA\\"}"
        env:
          STARLIST_URL: \${{ vars.STARLIST_URL }}
          STARLIST_TOKEN: \${{ secrets.STARLIST_TOKEN }}`;
	const markHook = `apiVersion: batch/v1
kind: Job
metadata:
  generateName: starlist-mark-
  annotations:
    argocd.argoproj.io/hook: PostSync                  # Argo CD
    # helm.sh/hook: post-install,post-upgrade          # or Helm
    # helm.sh/hook-delete-policy: hook-succeeded
spec:
  backoffLimit: 2
  template:
    spec:
      restartPolicy: Never
      containers:
        - name: mark
          image: curlimages/curl:8.10.1
          args:
            - -sf
            - -X
            - POST
            - http://api.shop.svc:7341/api/v1/markers
            - -H
            - "Authorization: Bearer $(STARLIST_TOKEN)"
            - -H
            - "Content-Type: application/json"
            - -d
            - '{"name": "v1.4.3", "deploy": true}'
          env:
            - name: STARLIST_TOKEN
              valueFrom: { secretKeyRef: { name: starlist, key: token } }`;
	const markLocal = `# On the same machine, against the collector's database
starlist mark v1.4.3 --deploy --note "fix currency KeyError" --db /data/starlist.db

# Or when starting starlist next to a new release
STARLIST_VERSION=v1.4.3 starlist serve --file app.log`;

	// Terraform
	const tfEcs = `resource "aws_ecs_task_definition" "api" {
  family                   = "api"
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = 512
  memory                   = 1024
  execution_role_arn       = aws_iam_role.task_execution.arn # can read the secret

  volume {
    name = "app-logs"
  }

  container_definitions = jsonencode([
    {
      name      = "api"
      image     = "\${var.app_image}:\${var.app_version}"
      essential = true
      # The app writes its log to /var/log/app/app.log
      mountPoints = [{ sourceVolume = "app-logs", containerPath = "/var/log/app" }]
    },
    {
      name      = "starlist"
      image     = "ghcr.io/p-lau/starlist:1"
      essential = false
      command   = ["serve", "--file=/var/log/app/app.log", "--http=0.0.0.0:7341", "--db=/data/starlist.db"]
      environment = [
        { name = "STARLIST_VERSION", value = var.app_version }, # records the deploy at startup
        { name = "STARLIST_ENV", value = "production" }
      ]
      secrets      = [{ name = "STARLIST_TOKEN", valueFrom = aws_secretsmanager_secret.starlist.arn }]
      portMappings = [{ containerPort = 7341 }]
      mountPoints  = [{ sourceVolume = "app-logs", containerPath = "/var/log/app", readOnly = true }]
    }
  ])
}`;
	const tfK8s = `# Reuse the sidecar manifest from the repository as is
resource "kubernetes_manifest" "api" {
  manifest = yamldecode(file("\${path.module}/sidecar.yaml"))
}`;
	const tfMark = `resource "terraform_data" "starlist_deploy" {
  # Runs again whenever the app's version changes
  triggers_replace = [var.app_version]
  depends_on       = [aws_ecs_service.api] # with wait_for_steady_state = true

  provisioner "local-exec" {
    command = <<-EOT
      curl -sf -X POST "$STARLIST_URL/api/v1/markers" \\
        -H "Authorization: Bearer $STARLIST_TOKEN" \\
        -H "Content-Type: application/json" \\
        -d '{"name": "\${var.app_version}", "deploy": true, "note": "terraform apply"}'
    EOT
    environment = {
      STARLIST_URL   = var.starlist_url
      STARLIST_TOKEN = var.starlist_token # a sensitive variable
    }
  }
}`;
	const tfCi = `# Keep each run's output, without color codes
terraform apply -auto-approve -no-color 2>&1 | tee apply.log

# When it fails, compare with the last successful run's apply.log
starlist-ci --baseline last-good/apply.log apply.log`;

	// Cloudflare
	const cfJq = `# workers.jq: one JSON line per console message and per exception
def ts: ./1000 | strftime("%Y-%m-%dT%H:%M:%SZ");
(.event.request.headers["cf-ray"] // null) as $ray
| (.logs[] | {ts: (.timestamp | ts),
              level: (if .level == "log" then "info" else .level end),
              msg: (.message | map(if type == "string" then . else tojson end) | join(" ")),
              ray: $ray}),
  (.exceptions[] | {ts: (.timestamp | ts), level: "error",
                    msg: "\\(.name): \\(.message)", ray: $ray})`;
	const cfTail = `npx wrangler tail shop-api --format json \\
  | jq -c --unbuffered -f workers.jq \\
  | starlist serve --stdin --request-key ray`;
	const cfLines = `{"ts":"2026-11-12T04:20:00Z","level":"error","msg":"pricing failed for user bob@example.com","ray":"8e1f2a3b4c5d6e7f"}
{"ts":"2026-11-12T04:20:00Z","level":"error","msg":"TypeError: Cannot read properties of undefined (reading 'currency')","ray":"8e1f2a3b4c5d6e7f"}`;
	const cfTunnel = `# starlist stays on localhost; with a token it accepts the tunnel's Host name
STARLIST_TOKEN=$(openssl rand -hex 32) starlist serve --file app.log

# Try it: a temporary https://….trycloudflare.com address
cloudflared tunnel --url http://127.0.0.1:7341`;
	const cfDeploy = `npx wrangler deploy && curl -sf -X POST "$STARLIST_URL/api/v1/markers" \\
  -H "Authorization: Bearer $STARLIST_TOKEN" -H "Content-Type: application/json" \\
  -d "{\\"name\\": \\"$(git rev-parse --short HEAD)\\", \\"deploy\\": true}"`;

	// Other platforms
	const platforms = `# Heroku
heroku logs --tail --app shop | starlist serve --stdin

# AWS CloudWatch Logs (a Lambda function, an ECS service...)
aws logs tail /aws/lambda/checkout --follow --format short | starlist serve --stdin

# Anything that writes to a terminal
./run-my-app 2>&1 | starlist serve --stdin --tee`;
	const ingest = `# Import old log files once (from the start), then ask about them
starlist ingest --db incident.db app.log.1 app.log
starlist mcp --db incident.db`;

	// Remote
	const caddy = `# Caddyfile: HTTPS in front of a collector on the same host
logs.example.com {
	reverse_proxy 127.0.0.1:7341
}`;
	const ssh = `ssh -N -L 7341:127.0.0.1:7341 app-host
# then connect the agent to http://127.0.0.1:7341/mcp as if it were local`;
</script>

<svelte:head>
	<title>{title}</title>
	<meta name="description" content={description} />
	<meta property="og:title" content={title} />
	<meta property="og:description" content={description} />
</svelte:head>

<article class="doc">
	<header>
		<p class="eyebrow">Docs · Integrations</p>
		<h1>Wherever your stars are.</h1>
		<p class="lede">
			How to run Starlist with the platforms and pipelines you already use. Each guide is marked
			<span class="kind builtin">built in</span> when Starlist supports it directly, or
			<span class="kind recipe">recipe</span> when it works by combining Starlist with the platform's
			own tools, like piping a log stream into <code>--stdin</code> or calling the JSON API from a
			pipeline.
		</p>
	</header>

	<div class="table">
		<table>
			<thead><tr><th>Where the logs are</th><th>How starlist reads them</th><th></th></tr></thead>
			<tbody>
				<tr><td>A file</td><td><code>--file PATH</code>, followed like <code>tail -F</code></td><td><span class="kind builtin">built in</span></td></tr>
				<tr><td>A process's output</td><td><code>app | starlist serve --stdin --tee</code></td><td><span class="kind builtin">built in</span></td></tr>
				<tr><td>Docker or Podman containers</td><td><code>--docker NAME</code>, <code>--docker-label K=V</code></td><td><span class="kind builtin">built in</span></td></tr>
				<tr><td>Kubernetes pods</td><td><code>--k8s deployment/NAME</code>, or a sidecar</td><td><span class="kind builtin">built in</span></td></tr>
				<tr><td>systemd journal</td><td><code>--journald UNIT</code></td><td><span class="kind builtin">built in</span></td></tr>
				<tr><td>OpenTelemetry SDKs and Collector</td><td><code>--otlp ADDR</code> (OTLP/HTTP)</td><td><span class="kind builtin">built in</span></td></tr>
				<tr><td>Loki and Grafana Cloud</td><td><code>--loki URL --loki-query Q</code></td><td><span class="kind builtin">built in</span></td></tr>
				<tr><td>CI job logs</td><td><code>starlist-ci</code>, GitHub Action</td><td><span class="kind builtin">built in</span></td></tr>
				<tr><td>Cloudflare Workers, Heroku, CloudWatch</td><td>The platform's tail command into <code>--stdin</code></td><td><span class="kind recipe">recipe</span></td></tr>
				<tr><td>GitLab CI, Jenkins, other CI</td><td><code>starlist-ci</code> in a job step</td><td><span class="kind recipe">recipe</span></td></tr>
				<tr><td>Deploys from any CD tool</td><td><code>POST /api/v1/markers</code></td><td><span class="kind recipe">recipe</span></td></tr>
				<tr><td>Terraform</td><td>A sidecar in your task or pod, and a marker on apply</td><td><span class="kind recipe">recipe</span></td></tr>
			</tbody>
		</table>
	</div>
	<p>
		Sources can be combined in one collector (<code>--file app.log --docker worker</code>), and every
		one goes through the same redaction, patterns and deploy detection. By default a source starts
		with lines written from now on; <code>--from-start</code> reads what it already has.
	</p>

	<!-- AGENTS -->
	<section id="agents">
		<p class="eyebrow">Receivers</p>
		<h2>MCP clients <span class="kind builtin">built in</span></h2>
		<p>
			<code>starlist init</code> writes these for the clients a project already uses. To do it by hand,
			point the client at <code>http://127.0.0.1:7341/mcp</code> (streamable HTTP) while
			<code>starlist serve</code> runs.
		</p>
		<Code title="Claude Code" code={claude} />
		<Code lang="json" title=".cursor/mcp.json (Cursor)" code={cursor} />
		<Code lang="json" title=".vscode/mcp.json (VS Code)" code={vscode} />
		<Code lang="json" title=".gemini/settings.json (Gemini CLI)" code={gemini} />
		<Code lang="json" title="~/.codeium/windsurf/mcp_config.json (Windsurf)" code={windsurf} />
		<Code title="Codex" code={codex} />
		<p>
			For a client that only speaks stdio, let it launch <code>starlist mcp --stdio</code>, which reads
			the database a running collector writes. <code>starlist init --stdio</code> sets that up.
		</p>
		<Code title="stdio" code={stdio} />
	</section>

	<!-- DOCKER -->
	<section id="docker">
		<p class="eyebrow">Containers</p>
		<h2>Docker and Compose <span class="kind builtin">built in</span></h2>
		<h3>Read a container's output</h3>
		<p>
			With the Docker CLI on the host, starlist runs <code>docker logs -f</code> for you. It follows a
			container through restarts and re-creation without reading lines twice, keeps stdout and stderr
			apart so their stack traces don't mix, and reads new containers under a label from their first
			line.
		</p>
		<Code title="host" code={dockerCli} />

		<h3>Run starlist in a container</h3>
		<p>
			The image <code>ghcr.io/p-lau/starlist:1</code> (amd64 and arm64) holds the static binary and a
			non-root user, with <code>/data</code> as its working directory and volume. Inside a container it
			listens beyond localhost, so it needs a token. Publish the port on <code>127.0.0.1</code> only.
		</p>
		<Code title="docker run" code={dockerRun} />
		<p>With Compose, share a log volume between the app and starlist:</p>
		<Code lang="yaml" title="compose.yaml" code={compose} />
		<p>
			The full file is <a href="{repo}/blob/main/deploy/compose.yaml" rel="noopener">deploy/compose.yaml</a>.
			Start it with <code>STARLIST_TOKEN=$(openssl rand -hex 32) docker compose up</code>.
		</p>

		<h3>Apps that only log to stdout</h3>
		<p>
			To read other containers from inside Compose, starlist needs the Docker CLI and the socket. The
			published image has only the binary, so add the CLI in a small image of your own:
		</p>
		<Code title="starlist/Dockerfile" code={dockerSock} />
		<Code lang="yaml" title="compose.yaml (excerpt)" code={dockerSockCompose} />
		<div class="note warn">
			<p>Anyone who can reach the Docker socket controls the host. Keep this container's port behind its token and on localhost.</p>
		</div>
	</section>

	<!-- KUBERNETES -->
	<section id="kubernetes">
		<p class="eyebrow">Fleets</p>
		<h2>Kubernetes <span class="kind builtin">built in</span></h2>
		<p>There are two ways in: a sidecar next to each app, or one collector that follows a workload's pods.</p>

		<h3>Sidecar</h3>
		<p>
			The app writes its log to a shared <code>emptyDir</code>, and starlist follows the file. The
			sidecar asks for 10m of CPU and 32Mi of memory. Each replica keeps its own database; add a
			<a href="/docs/#hub">hub</a> to see them together.
		</p>
		<Code lang="yaml" title="deployment.yaml" code={sidecar} />
		<Code title="apply and connect" code={sidecarRun} />
		<p>The full manifest is <a href="{repo}/blob/main/deploy/kubernetes/sidecar.yaml" rel="noopener">deploy/kubernetes/sidecar.yaml</a>.</p>

		<h3>Follow a workload's pods</h3>
		<p>
			With <code>--k8s</code>, starlist uses kubectl to look up the workload's selector, list its pods
			every 5 seconds and follow each container's log. New pods from a rollout, scale-up or rescheduling
			are read from their first line, and a crash-looping container is followed from where it left off.
		</p>
		<Code title="from your machine" code={k8sCli} />
		<p>
			It uses your current kubectl context. To run it in the cluster, give its service account this
			Role, and use an image that has kubectl, since the published one has only the binary:
		</p>
		<Code lang="yaml" title="role.yaml" code={rbac} />
		<Code title="Dockerfile" code={kubectlImage} />
		<p>Bind the Role to the service account with a RoleBinding, and set <code>STARLIST_KUBECTL</code> if kubectl lives elsewhere.</p>
	</section>

	<!-- SYSTEMD -->
	<section id="systemd">
		<p class="eyebrow">Hosts</p>
		<h2>systemd and journald <span class="kind builtin">built in</span></h2>
		<p>
			starlist runs <code>journalctl --follow --output=json</code> for each unit and uses each entry's own
			time. A stack trace written line by line is put back together into one event, and an entry's
			journal priority sets its level when the message doesn't carry one.
		</p>
		<Code title="journald" code={journald} />
		<p>
			To run starlist itself as a service, give it a user in the <code>systemd-journal</code> group so it
			can read the journal:
		</p>
		<Code lang="toml" title="starlist.service" code={unit} />
		<Code title="enable" code={unitRun} />
	</section>

	<!-- OTEL -->
	<section id="opentelemetry">
		<p class="eyebrow">Telemetry</p>
		<h2>OpenTelemetry <span class="kind builtin">built in</span></h2>
		<p>
			starlist accepts OTLP logs over HTTP at <code>/v1/logs</code>, in protobuf or JSON, optionally
			gzipped. Each record's severity becomes the level, its body the message, and its trace ID is kept,
			so <code>get_request</code> with a trace ID returns the whole trace. Exception attributes become
			the stack trace, which gives the pattern its type and code location.
		</p>
		<Code title="SDK" code={otlp} />
		<p>Or send a copy from an OpenTelemetry Collector you already run:</p>
		<Code lang="yaml" title="collector.yaml" code={otelCollector} />
		<p>
			Listening beyond localhost needs <code>--otlp-token</code> (or <code>STARLIST_OTLP_TOKEN</code>, 16
			characters or more). gRPC on port 4317 isn't supported; use OTLP/HTTP.
		</p>
	</section>

	<!-- LOKI -->
	<section id="loki">
		<p class="eyebrow">Existing pipelines</p>
		<h2>Loki and Grafana <span class="kind builtin">built in</span></h2>
		<p>
			If Loki already holds your logs, starlist can read a LogQL stream and give agents patterns,
			<code>whats_new</code> and <code>verify_fix</code> over it, without changing how logs are shipped.
			It polls <code>query_range</code> every 2 seconds and looks back 15 seconds more for late lines.
		</p>
		<Code title="Loki" code={loki} />
		<p>
			The query must be a log query (a selector, optionally with filters like <code>|= "error"</code>),
			not a metric query. <code>--loki-org</code> sets <code>X-Scope-OrgID</code> for a multi-tenant Loki.
			Grafana dashboards can also scrape starlist's own numbers with <code>--prometheus</code>.
		</p>
	</section>

	<!-- GITHUB ACTIONS -->
	<section id="github-actions">
		<p class="eyebrow">CI</p>
		<h2>GitHub Actions <span class="kind builtin">built in</span> <span class="kind soon">next release</span></h2>
		<p>
			The <code>p-lau/starlist/ci</code> action keeps each run's log as an artifact. When the job fails,
			it compares the log with the last passing run of the same workflow on the pull request's base
			branch, and writes the report to the job summary and, if asked, a pull request comment that is
			updated on later failures. It never fails the job itself.
		</p>
		<Code lang="yaml" title=".github/workflows/test.yml" code={gha} />
		<div class="note warn">
			<p>
				The action and <code>starlist-ci</code> are merged but not released yet, so there is no
				<code>v1.1.0</code> tag or release binary. Until then, install <code>starlist-ci</code> in the job
				(the action uses one on <code>PATH</code>) and point at <code>main</code>:
			</p>
			<Code lang="yaml" title="until the release" code={ghaNow} />
		</div>
		<p>
			Give each matrix entry its own artifact with <code>name: starlist-log-$&#123;&#123; matrix.os &#125;&#125;</code>.
			An agent fixing a red build can also run the comparison itself:
		</p>
		<Code title="agent" code={ghaAgent} />
		<p>All inputs are in <a href="{repo}/blob/main/ci/README.md" rel="noopener">ci/README.md</a>.</p>
	</section>

	<!-- GITLAB -->
	<section id="gitlab">
		<p class="eyebrow">CI</p>
		<h2>GitLab CI <span class="kind recipe">recipe</span></h2>
		<p>
			Keep the job's log as an artifact on every run. When it fails, <code>after_script</code> downloads
			the same file from the latest successful pipeline on the default branch and compares. The token
			in <code>STARLIST_READ_TOKEN</code> needs <code>read_api</code> (a project access token works);
			public projects can drop the header.
		</p>
		<Code lang="yaml" title=".gitlab-ci.yml" code={gitlab} />
		<p>
			The report lands in the job log and in the <code>starlist-ci.md</code> artifact. For an image
			without Rust, download a <code>starlist-ci</code> release archive instead once it is released.
		</p>
	</section>

	<!-- JENKINS -->
	<section id="jenkins">
		<p class="eyebrow">CI</p>
		<h2>Jenkins <span class="kind recipe">recipe</span></h2>
		<p>
			Archive the log on every build, and on failure fetch it from the last successful build with the
			<a href="https://plugins.jenkins.io/copyartifact/" rel="noopener">Copy Artifact</a> plugin. Install
			<code>starlist-ci</code> on the agents (or in the agent image).
		</p>
		<Code lang="groovy" title="Jenkinsfile" code={jenkins} />
		<p>
			If Copy Artifact refuses, let the job copy from itself with
			<code>options &#123; copyArtifactPermission('your-job') &#125;</code>. Without the plugin, download
			the artifact with a user's API token:
		</p>
		<Code title="without the plugin" code={jenkinsCurl} />
	</section>

	<!-- OTHER CI -->
	<section id="other-ci">
		<p class="eyebrow">CI</p>
		<h2>Other CI systems <span class="kind recipe">recipe</span></h2>
		<p>
			CircleCI, Buildkite, Azure Pipelines, Drone and the rest all work the same way: keep each run's
			log, and when a run fails, compare it with the last green one's.
		</p>
		<Code title="any CI" code={genericCi} />
		<p>
			<code>starlist-ci</code> reads plain output from any tool. Without a passing log it lists the
			errors and warnings instead, with line numbers, which is already a smaller read than the whole log.
		</p>
	</section>

	<!-- DEPLOYS -->
	<section id="deploys">
		<p class="eyebrow">Launch log</p>
		<h2>Deploy markers from CD <span class="kind recipe">recipe</span></h2>
		<p>
			starlist already picks up deploys from startup lines like <code>Starting api version 1.4.2</code>.
			When your app doesn't log its version, have the pipeline record the release. Then
			<code>compare_deploy</code>, <code>verify_fix</code> with <code>since: "v1.4.3"</code>,
			<code>get_release</code> and every pattern's <code>first_release</code> know about it.
		</p>
		<h3>From any pipeline: the JSON API</h3>
		<Code title="curl" code={markCurl} />
		<Code lang="text" title="answer" code={markAnswer} />
		<h3>GitHub Actions deploy job</h3>
		<Code lang="yaml" title="after the deploy step" code={markGha} />
		<h3>Argo CD or Helm hook</h3>
		<p>A Job that runs after each sync or upgrade and posts the marker to the sidecar's service:</p>
		<Code lang="yaml" title="mark-deploy.yaml" code={markHook} />
		<p>Template the release name from your chart's image tag or the Argo CD revision.</p>
		<h3>On the same machine</h3>
		<Code title="local" code={markLocal} />
	</section>

	<!-- TERRAFORM -->
	<section id="terraform">
		<p class="eyebrow">Infrastructure</p>
		<h2>Terraform <span class="kind recipe">recipe</span></h2>
		<p>
			There is no Terraform provider for starlist, and none is needed: Terraform doesn't write the logs,
			it deploys what does. It fits in three places.
		</p>

		<h3>Deploy starlist next to the app</h3>
		<p>
			Add starlist as a second container wherever your Terraform already defines the app. On ECS
			Fargate, the two containers share a task volume for the log file, and the token comes from
			Secrets Manager:
		</p>
		<Code lang="hcl" title="ecs.tf" code={tfEcs} />
		<p>
			Setting <code>STARLIST_VERSION</code> from the same variable as the image tag records each release
			as a deploy when the new task starts, so nothing else is needed for markers. The database lives in
			the task and starts empty when the task is replaced; mount an EFS volume at <code>/data</code> to
			keep history. Reach port 7341 over a private network or an HTTPS load balancer, never in the open.
		</p>
		<p>On Kubernetes, the manifest from the <a href="#kubernetes">Kubernetes</a> guide works with the kubernetes provider unchanged:</p>
		<Code lang="hcl" title="k8s.tf" code={tfK8s} />

		<h3>Record deploys from terraform apply</h3>
		<p>
			When starlist runs apart from the app (a long-running collector, or one reading Docker or
			Kubernetes logs), have the apply post a marker once the new version is live:
		</p>
		<Code lang="hcl" title="marker.tf" code={tfMark} />
		<p>
			Set <code>wait_for_steady_state = true</code> on the <code>aws_ecs_service</code> (or wait for the
			rollout on other platforms), or the marker lands before the new version is serving.
		</p>

		<h3>When terraform apply fails in CI</h3>
		<p>
			An apply's output is a log like any other. Keep it on every run, and on a failure compare it with
			the last successful one, as in the <a href="#other-ci">other CI</a> recipe:
		</p>
		<Code title="CI" code={tfCi} />
	</section>

	<!-- CLOUDFLARE -->
	<section id="cloudflare">
		<p class="eyebrow">Edge</p>
		<h2>Cloudflare <span class="kind recipe">recipe</span></h2>
		<p>
			starlist has no Cloudflare connector, but three pieces fit together well: Workers logs through
			<code>wrangler tail</code>, Cloudflare Tunnel for reaching a collector over HTTPS, and deploy
			markers from <code>wrangler deploy</code>.
		</p>

		<h3>Workers logs</h3>
		<p>
			<code>wrangler tail --format json</code> prints one JSON object per request, with that request's
			console messages and exceptions inside. A short <code>jq</code> filter turns each message into a
			line starlist reads, with the request's <code>cf-ray</code> as its request ID, so
			<code>get_request</code> shows everything one request logged.
		</p>
		<Code title="workers.jq" code={cfJq} />
		<Code title="follow a Worker" code={cfTail} />
		<p>What starlist receives (emails are redacted before they're stored):</p>
		<Code lang="json" title="lines" code={cfLines} wrap />
		<p>
			Keep it running while you work, in a terminal or on a small always-on machine. Under heavy traffic
			<code>wrangler tail</code> samples requests, so counts are a sample too; patterns,
			<code>whats_new</code> and <code>verify_fix</code> still work on the rates.
		</p>

		<h3>Reach a collector through a Tunnel</h3>
		<p>
			Cloudflare Tunnel gives a collector an HTTPS address without opening a port. Set a token: starlist
			then accepts the tunnel's Host name, and agents send the token. For anything lasting, use a named
			tunnel and put a Cloudflare Access policy in front as well.
		</p>
		<Code title="tunnel" code={cfTunnel} />

		<h3>Mark Worker deploys</h3>
		<Code title="deploy" code={cfDeploy} />
	</section>

	<!-- PLATFORMS -->
	<section id="platforms">
		<p class="eyebrow">Elsewhere</p>
		<h2>Other platforms <span class="kind recipe">recipe</span></h2>
		<p>
			Any platform with a command that streams logs can feed <code>--stdin</code>. Lines with a timestamp
			keep it; lines without one get the time they were read.
		</p>
		<Code title="stdin" code={platforms} />
		<p>For logs from an incident that already happened, import the files and ask about them afterwards:</p>
		<Code title="ingest" code={ingest} />
	</section>

	<!-- REMOTE -->
	<section id="remote">
		<p class="eyebrow">Uplink</p>
		<h2>Reaching it remotely</h2>
		<p>
			starlist has no TLS of its own. Keep it on <code>127.0.0.1</code> and put something with HTTPS in
			front, or tunnel to it. Behind a proxy or tunnel, set <code>--token</code> (or
			<code>STARLIST_TOKEN</code>): without one, starlist answers only loopback Host names, which blocks
			DNS rebinding but also the proxy's own Host name.
		</p>
		<Code title="Caddyfile" code={caddy} />
		<Code title="SSH tunnel" code={ssh} />
		<p>
			The same applies to a <a href="/docs/#hub">hub</a>. Collectors warn when they push to plain
			<code>http://</code> beyond localhost.
		</p>
	</section>
</article>
