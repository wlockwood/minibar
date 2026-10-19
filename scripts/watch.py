#!/usr/bin/env python3
"""Local server for the council viewer: watch battles at your own speed.

Serves viewer/brain_watch.html and plays battles on demand with the simulator,
so the page can show any scenario, opponent and seed. Two ways to point it at a
brain:

  python scripts/watch.py --catalog examples/pawn-hello.json --dir data/brains/evolve-pawn-skirmish
      follows an `evolve.py` run: the page notices when a new champion is
      promoted, shows the evolution status, and reloads on its own.

  python scripts/watch.py --catalog examples/pawn-hello.json \\
      --brain data/brains/x/brain.best.json --train-config data/brains/x/train_config.json
      watches one fixed brain.

Binds to 127.0.0.1 only.
"""

import argparse
import json
import os
import subprocess
import sys
import threading
import webbrowser
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import parse_qs, urlparse

HERE = Path(__file__).resolve().parent.parent
VIEWER = HERE / "viewer" / "brain_watch.html"


class Source:
    """Where the current brain and config come from, and what has changed."""

    def __init__(self, brain, config, evolve_dir):
        self.brain, self.config, self.dir = Path(brain), Path(config), evolve_dir
        self.lock = threading.Lock()
        self.cache = {}
        self.cache_version = None

    def version(self):
        try:
            st = self.brain.stat()
            return f"{st.st_mtime_ns}-{st.st_size}"
        except FileNotFoundError:
            return "none"

    def status(self):
        """Evolution progress for the page header, or None for a fixed brain."""
        if not self.dir:
            return None
        try:
            state = json.loads((self.dir / "state.json").read_text(encoding="utf-8"))
        except (FileNotFoundError, json.JSONDecodeError):
            return None
        champion = state.get("champion")
        if not champion:
            return {"round": state.get("round", 0), "champion_round": "-", "score": 0.0}
        status = {
            "round": state.get("round", 0),
            "champion_round": champion["round"],
            "score": champion.get("score", 0.0),
        }
        try:  # latest history line for the champion's win rates
            last = (self.dir / "history.jsonl").read_text(encoding="utf-8").strip().splitlines()[-1]
            record = json.loads(last)
            candidate = record["candidates"].get("champion") or record["candidates"].get(record["winner"])
            if candidate:
                status["baseline_win"] = candidate["baseline_win"]
                status["underdog_win"] = candidate["underdog_win"]
        except (FileNotFoundError, IndexError, KeyError, json.JSONDecodeError):
            pass
        return status


def make_handler(source, exe, catalog, unit):
    def run(args, timeout=60):
        cmd = [exe, "--catalog", catalog, "--unit", unit, *map(str, args)]
        done = subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8", timeout=timeout,
                              env=dict(os.environ, RAYON_NUM_THREADS="2"))
        if done.returncode != 0:
            raise RuntimeError(done.stderr.strip() or "simulator failed")
        return done.stdout

    class Handler(BaseHTTPRequestHandler):
        def log_message(self, *args):  # keep the terminal quiet
            pass

        def send(self, code, body, kind="application/json"):
            data = body.encode("utf-8") if isinstance(body, str) else body
            self.send_response(code)
            self.send_header("Content-Type", kind + "; charset=utf-8")
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            url = urlparse(self.path)
            query = {k: v[0] for k, v in parse_qs(url.query).items()}
            try:
                if url.path in ("/", "/index.html"):
                    self.send(200, VIEWER.read_text(encoding="utf-8"), "text/html")
                elif url.path == "/api/version":
                    self.send(200, json.dumps({"version": source.version(), "status": source.status()}))
                elif url.path == "/api/meta":
                    self.send(200, run(["--watch-meta", source.brain, "--train-config", source.config,
                                        "--watch-title", source.config.stem if not source.dir else source.dir.name]))
                elif url.path == "/api/battle":
                    args = ["--watch-battle", source.brain, "--train-config", source.config,
                            "--watch-scenario", int(query.get("scenario", 0)),
                            "--watch-opponent", int(query.get("opponent", 0)),
                            "--watch-seed", int(query.get("seed", 1)),
                            "--watch-side", int(query.get("side", 0))]
                    key = (source.version(), tuple(map(str, args[5:])))
                    with source.lock:
                        if source.cache_version != source.version():
                            source.cache, source.cache_version = {}, source.version()
                        body = source.cache.get(key)
                    if body is None:
                        body = run(args)
                        with source.lock:
                            if len(source.cache) > 24:
                                source.cache.pop(next(iter(source.cache)))
                            source.cache[key] = body
                    self.send(200, body)
                else:
                    self.send(404, json.dumps({"error": "not found"}))
            except (ValueError, KeyError) as error:
                self.send(400, str(error), "text/plain")
            except Exception as error:  # surfaced in the page's overlay
                self.send(500, str(error), "text/plain")

    return Handler


def main():
    exe_name = "visigoth-simulator.exe" if os.name == "nt" else "visigoth-simulator"
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--catalog", required=True)
    parser.add_argument("--unit", default="armpw")
    parser.add_argument("--exe", default=str(HERE / "target" / "release" / exe_name))
    parser.add_argument("--dir", help="an evolve.py output directory (follows its champion)")
    parser.add_argument("--brain", help="a fixed brain file")
    parser.add_argument("--train-config", help="config naming scenarios and opponents (with --brain)")
    parser.add_argument("--port", type=int, default=8765)
    parser.add_argument("--no-browser", action="store_true")
    args = parser.parse_args()

    if not Path(args.exe).exists():
        sys.exit(f"simulator binary not found at {args.exe}; run `cargo build --release` or pass --exe")
    if args.dir:
        directory = Path(args.dir)
        brain, config = directory / "champion.json", directory / "base_config.json"
    elif args.brain and args.train_config:
        directory, brain, config = None, Path(args.brain), Path(args.train_config)
    else:
        sys.exit("give --dir EVOLVE_DIR, or --brain FILE --train-config FILE")

    source = Source(brain, config, directory)
    if not brain.exists():
        print(f"waiting for the first champion at {brain} ...")
    server = ThreadingHTTPServer(("127.0.0.1", args.port), make_handler(source, args.exe, args.catalog, args.unit))
    url = f"http://127.0.0.1:{server.server_address[1]}/"
    print(f"council viewer at {url}  (Ctrl+C to stop)")
    if not args.no_browser:
        threading.Timer(0.5, lambda: webbrowser.open(url)).start()
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
