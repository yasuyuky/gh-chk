#!/usr/bin/env python3

import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlsplit
from uuid import uuid4


def resolve_data_dir():
    script_dir = Path(__file__).resolve().parent
    for path in [script_dir / "data", script_dir.parent / "data"]:
        if path.is_dir():
            return path
    raise FileNotFoundError(f"fixture directory not found near {script_dir}")


DATA_DIR = resolve_data_dir()


def load_json(name: str):
    return json.loads((DATA_DIR / name).read_text())


def dependabot_alert_pr_page(pr_id: str, end_cursor=None, has_next_page=False):
    return {
        "data": {
            "repository": {
                "vulnerabilityAlerts": {
                    "nodes": [
                        {
                            "dependabotUpdate": {
                                "pullRequest": {
                                    "id": pr_id
                                }
                            }
                        }
                    ],
                    "pageInfo": {
                        "endCursor": end_cursor,
                        "hasNextPage": has_next_page,
                    },
                }
            }
        }
    }


def response_for(scenario: str, payload: dict):
    if payload.get("operationName") == "MergePullRequest":
        return 400, {"message": "use the async merge REST API"}
    if payload.get("operationName") == "GetDependabotAlertPullRequestIds":
        if scenario == "prs_dependabot_alert_error":
            return 500, {"message": "dependabot alert lookup failed"}
        if scenario == "prs_dependabot_alert_short_circuit":
            after = (payload.get("variables") or {}).get("after")
            if after is None:
                return 200, dependabot_alert_pr_page("PRID1", "alert-page-1", True)
            return 500, {"message": f"unexpected alert page request: {after}"}
        if scenario == "prs_dependabot_alert":
            return 200, load_json("dependabot_alert_prs.json")
        return 200, load_json("dependabot_alert_prs_empty.json")

    if scenario == "issues":
        return 200, load_json("issues.json")

    if scenario == "prs" or scenario.startswith("merge_"):
        body = load_json("prs.json")
        if scenario == "merge_blocked":
            body["data"]["search"]["nodes"][0]["mergeStateStatus"] = "BLOCKED"
        return 200, body

    if scenario == "prs_dependabot_alert":
        return 200, load_json("prs.json")

    if scenario == "prs_dependabot_alert_error":
        return 200, load_json("prs.json")

    if scenario == "prs_dependabot_alert_short_circuit":
        return 200, load_json("prs.json")

    if scenario == "prs_requested_reviewers":
        return 200, load_json("prs_requested_reviewers.json")

    if scenario == "prs_paginated":
        after = (payload.get("variables") or {}).get("after")
        pages = load_json("prs_paginated.json")
        if after is None:
            return 200, pages[0]
        if after == "cursor-page-1":
            return 200, pages[1]
        return 400, {"error": f"unknown cursor: {after}"}

    return 500, {"error": f"unknown scenario: {scenario}"}


class Handler(BaseHTTPRequestHandler):
    timezones = {}
    merge_requests = {}

    def do_GET(self):
        if self.path.startswith("/rest/"):
            self.handle_merge()
            return
        if self.path != "/healthz":
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Length", "2")
        self.end_headers()
        self.wfile.write(b"ok")

    def do_PUT(self):
        self.handle_merge()

    def handle_merge(self):
        parts = urlsplit(self.path).path.split("/")
        if len(parts) not in (9, 10) or parts[1] != "rest":
            self.send_error(404)
            return
        scenario = parts[2]
        if parts[3:9] != ["repos", "owner1", "repo1", "pulls", "1", "merge-async"]:
            self.send_error(404)
            return
        expected_headers = {
            "Accept": "application/vnd.github+json",
            "X-GitHub-Api-Version": "2026-03-10",
        }
        if scenario in self.timezones:
            expected_headers["Time-Zone"] = self.timezones[scenario]
        for name, expected in expected_headers.items():
            if self.headers.get(name) != expected:
                self.send_json(400, {"message": f"unexpected {name} header"})
                return
        if not self.headers.get("Authorization", "").startswith("bearer "):
            self.send_json(401, {"message": "missing bearer token"})
            return
        responses = load_json("merge_async.json")
        if self.command == "PUT" and len(parts) == 9:
            length = int(self.headers.get("Content-Length", "0"))
            payload = json.loads(self.rfile.read(length) or b"{}")
            if payload != {"merge_action": "default", "bypass_rules": False}:
                self.send_json(400, {"message": "unexpected merge options"})
                return
            if scenario in ("merge_forbidden", "merge_blocked"):
                self.send_json(403, {"message": "Resource not accessible"})
                return
            if scenario == "merge_rejected":
                self.send_json(400, responses["failed"])
                return
            if scenario == "merge_conflict_error":
                self.send_json(409, {"message": "Conflicting merge options"})
                return
            if scenario == "merge_invalid":
                self.send_json(202, {"status": "pending", "details": {}})
                return
            if scenario.startswith("merge_already_"):
                self.send_json(200, responses[scenario.removeprefix("merge_already_")])
                return
            request_id = str(uuid4())
            self.merge_requests[request_id] = [scenario, 0]
            responses["pending"]["details"]["uuid"] = request_id
            self.send_json(409 if scenario == "merge_conflict" else 202, responses["pending"])
            return
        if self.command == "GET" and len(parts) == 10:
            request_id = parts[9]
            request = self.merge_requests.get(request_id)
            if request is None or request[0] != scenario:
                self.send_json(404, {"message": "unknown merge request ID"})
                return
            if scenario == "merge_poll_error":
                self.send_json(404, {"message": "Merge request expired"})
                return
            request[1] += 1
            result = "merged"
            if request[1] == 1:
                result = "pending"
            elif scenario == "merge_enqueued":
                result = "enqueued"
            elif scenario == "merge_failed":
                result = "failed"
            responses["pending"]["details"]["uuid"] = request_id
            self.send_json(200, responses[result])
            return
        self.send_error(405)

    def do_POST(self):
        prefix = "/graphql/"
        url = urlsplit(self.path)
        if not url.path.startswith(prefix):
            self.send_error(404)
            return

        try:
            scenario = url.path[len(prefix):]
            length = int(self.headers.get("Content-Length", "0"))
            payload = json.loads(self.rfile.read(length) or b"{}")
            timezone = parse_qs(url.query).get("time_zone", [None])[0]
            if timezone is not None:
                self.timezones[scenario] = timezone
            if timezone is not None and self.headers.get("Time-Zone") != timezone:
                status, body = 400, {"message": "unexpected Time-Zone header"}
            else:
                status, body = response_for(scenario, payload)
        except Exception as err:
            status, body = 500, {"error": str(err)}

        self.send_json(status, body)

    def send_json(self, status, body):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, fmt, *args):
        return


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, required=True)
    args = parser.parse_args()

    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    server.serve_forever()


if __name__ == "__main__":
    main()
