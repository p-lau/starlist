"""A one-tool MCP server for the benchmark's `subagent` arm.

The main agent gets only `investigate_logs`. Each call starts a separate headless agent
that has starlist's tools and nothing else (no code, no shell), and returns its short
answer. So the main agent never carries starlist's tool definitions or raw findings
through its turns; it pays for one small tool and the answers it asks for.

Claude Code's own subagents could do this with an agent-scoped MCP server, but not
everywhere (some environments skip agent-defined MCP servers), and a separate process
also lets the runner measure the log agent's tokens on their own.

    python3 subagent_mcp.py --mcp-config work/mcp.json --work work --model claude-sonnet-5-5

Speaks MCP over stdio (newline-delimited JSON-RPC). Each log agent's stream is saved as
work/sub-<n>.jsonl for run.py to measure.
"""

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

TOOL = {
    "name": "investigate_logs",
    "description": "Ask a log agent about the service's production logs. It can't see the code. "
                   "Returns the likely cause, its code location, the deploy it started with and key log lines.",
    "inputSchema": {
        "type": "object",
        "properties": {"question": {"type": "string"}},
        "required": ["question"],
    },
}

PROMPT = """You investigate the shop service's production logs with the starlist tools. You can't see or change the code.

Question from the engineer fixing the service: {question}

Start with diagnose. Reply in under 150 words: the likely cause, its pattern ID, the code location, the deploy it started with if any, and the one or two log lines that show it, quoted exactly. If the logs don't show a cause, say what they do show."""


def ask(args, question: str, n: int) -> str:
    out = Path(args.work) / f"sub-{n}.jsonl"
    cmd = [os.environ.get("CLAUDE_BIN", "claude"), "-p", PROMPT.format(question=question),
           "--output-format", "stream-json", "--verbose", "--model", args.model,
           "--setting-sources", "", "--max-turns", str(args.max_turns),
           "--tools", "", "--strict-mcp-config", "--mcp-config", args.mcp_config,
           "--allowedTools", "mcp__starlist"]
    with open(out, "w") as f:
        try:
            subprocess.run(cmd, stdout=f, stderr=subprocess.STDOUT, stdin=subprocess.DEVNULL,
                           timeout=args.timeout, cwd=args.work)
        except subprocess.TimeoutExpired:
            return "The log agent timed out."
    answer = ""
    for line in out.read_text().splitlines():
        try:
            ev = json.loads(line)
        except json.JSONDecodeError:
            continue
        if ev.get("type") == "result":
            answer = ev.get("result") or ""
    return answer or "The log agent gave no answer."


def main() -> None:
    p = argparse.ArgumentParser()
    p.add_argument("--mcp-config", required=True, help="MCP config with the starlist server")
    p.add_argument("--work", required=True)
    p.add_argument("--model", default="claude-sonnet-5-5")
    p.add_argument("--max-turns", type=int, default=15)
    p.add_argument("--timeout", type=int, default=600)
    args = p.parse_args()
    calls = 0

    def reply(id_, result=None, error=None):
        msg = {"jsonrpc": "2.0", "id": id_}
        if error:
            msg["error"] = error
        else:
            msg["result"] = result
        sys.stdout.write(json.dumps(msg) + "\n")
        sys.stdout.flush()

    for line in sys.stdin:
        try:
            req = json.loads(line)
        except json.JSONDecodeError:
            continue
        method, id_ = req.get("method"), req.get("id")
        if id_ is None:  # notifications
            continue
        if method == "initialize":
            reply(id_, {
                "protocolVersion": req.get("params", {}).get("protocolVersion", "2025-06-18"),
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "logs", "version": "0.1.0"},
            })
        elif method == "tools/list":
            reply(id_, {"tools": [TOOL]})
        elif method == "tools/call":
            params = req.get("params", {})
            if params.get("name") != TOOL["name"]:
                reply(id_, error={"code": -32602, "message": f"unknown tool {params.get('name')}"})
                continue
            calls += 1
            question = (params.get("arguments") or {}).get("question", "What is wrong?")
            text = ask(args, question, calls)
            reply(id_, {"content": [{"type": "text", "text": text}]})
        elif method == "ping":
            reply(id_, {})
        else:
            reply(id_, error={"code": -32601, "message": f"unknown method {method}"})


if __name__ == "__main__":
    main()
