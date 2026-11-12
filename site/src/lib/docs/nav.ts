// The docs sidebar: each page and the sections on it.

export type NavPage = { href: string; title: string; sections: { id: string; title: string }[] };

export const docsNav: NavPage[] = [
	{
		href: '/docs/',
		title: 'Guide',
		sections: [
			{ id: 'start', title: 'Get started' },
			{ id: 'concepts', title: 'How starlist reads logs' },
			{ id: 'tools', title: 'Tools' },
			{ id: 'walkthrough', title: 'Walkthrough: a bad deploy' },
			{ id: 'alerts', title: 'Alerts' },
			{ id: 'dashboard', title: 'Dashboard and JSON API' },
			{ id: 'history', title: 'History, metrics and export' },
			{ id: 'hub', title: 'Hub for many services' },
			{ id: 'ci', title: 'starlist-ci for CI logs' },
			{ id: 'config', title: 'Configuration' },
			{ id: 'security', title: 'Security' }
		]
	},
	{
		href: '/docs/integrations/',
		title: 'Integrations',
		sections: [
			{ id: 'agents', title: 'MCP clients' },
			{ id: 'docker', title: 'Docker and Compose' },
			{ id: 'kubernetes', title: 'Kubernetes' },
			{ id: 'systemd', title: 'systemd and journald' },
			{ id: 'opentelemetry', title: 'OpenTelemetry' },
			{ id: 'loki', title: 'Loki and Grafana' },
			{ id: 'github-actions', title: 'GitHub Actions' },
			{ id: 'gitlab', title: 'GitLab CI' },
			{ id: 'jenkins', title: 'Jenkins' },
			{ id: 'other-ci', title: 'Other CI systems' },
			{ id: 'deploys', title: 'Deploy markers from CD' },
			{ id: 'terraform', title: 'Terraform' },
			{ id: 'cloudflare', title: 'Cloudflare' },
			{ id: 'platforms', title: 'Other platforms' },
			{ id: 'remote', title: 'Reaching it remotely' }
		]
	}
];
