#!/usr/bin/env python3

import argparse
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path


def resolve_data_dir():
    script_dir = Path(__file__).resolve().parent
    for path in [script_dir / "data", script_dir.parent / "data"]:
        if path.is_dir():
            return path
    raise FileNotFoundError(f"fixture directory not found near {script_dir}")


DATA_DIR = resolve_data_dir()


def load_json(name: str):
    return json.loads((DATA_DIR / name).read_text())


def response_for(scenario: str, payload: dict):
    if scenario == "issues":
        return 200, load_json("issues.json")

    if scenario == "prs":
        return 200, load_json("prs.json")

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

    def do_GET(self):
        if self.path != "/healthz":
            self.send_error(404)
            return
        self.send_response(200)
        self.send_header("Content-Length", "2")
        self.end_headers()
        self.wfile.write(b"ok")

    def do_POST(self):
        prefix = "/graphql/"
        if not self.path.startswith(prefix):
            self.send_error(404)
            return

        try:
            scenario = self.path[len(prefix):]
            length = int(self.headers.get("Content-Length", "0"))
            payload = json.loads(self.rfile.read(length) or b"{}")
            status, body = response_for(scenario, payload)
        except Exception as err:
            status, body = 500, {"error": str(err)}

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
