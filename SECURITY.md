# Security policy

starlist reads application logs, which can hold secrets and personal data, so security reports are very welcome.

## Supported versions

Until 1.0.0, only the latest release gets security fixes. From 1.0.0, fixes go to the latest minor release of the current major version.

## Reporting a vulnerability

Please don't open a public issue. Report it privately through [GitHub's private vulnerability reporting](https://github.com/p-lau/starlist/security/advisories/new) instead.

Include what you can of:

- the version (`starlist --version`) or commit, and how you run it (flags, features, platform);
- what an attacker can do, and from where (local user, same network, a web page in the browser, a collector pushing to a hub);
- steps or a log sample that reproduce it, with any real secrets replaced.

You should get a reply within a week. Once a fix is ready, it ships in a release with a GitHub security advisory, and you're credited unless you'd rather not be.

## What counts

Things we'd treat as vulnerabilities include:

- a secret or personal value that the built-in redaction is meant to catch (see the README) reaching the database, the MCP tools, the API, the dashboard or a hub;
- reading or writing through the MCP endpoint, API, dashboard, metrics or hub without the right token, or with another service's push token;
- getting past the loopback `Host` check or the cross-site write check from a browser;
- the token file or database being created readable by other users;
- a crafted log line, OTLP request or hub push that crashes starlist, or makes it use unbounded memory or disk.

Redaction can't know every sensitive value an app logs, so a value that isn't one of the documented kinds is a feature request (or a case for `--redact`), not a vulnerability. The same goes for running beyond localhost without TLS: put starlist behind a TLS-terminating proxy, as the README describes.
