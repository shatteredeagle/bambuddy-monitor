#!/usr/bin/env python3
"""Tiny scripted Bambuddy for testing alerts (stdlib only).

    python3 tests/fake_bambuddy.py [port]      # default 8790
    curl localhost:8790/_step/2                # move the printer to scenario step 2

Steps: 0 printing, 1 spool in use low, 2 paused + out of filament, 3 resumed, 4 finished, 5 offline.
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

step = 0


def status():
    s = {
        "id": 1, "name": "P1S", "connected": step < 5, "state": "RUNNING", "subtask_name": "Benchy",
        "progress": 40.0, "remaining_time": 80, "layer_num": 60, "total_layers": 160, "hms_errors": [],
        "temperatures": {"nozzle": 220, "bed": 55, "chamber": 30}, "tray_now": 1,
        "ams": [{"id": 0, "tray": [{"id": 0, "remain": 80}, {"id": 1, "remain": 10 if step >= 1 else 60}]}],
    }
    if step == 2:
        s["state"] = "PAUSE"
        s["hms_errors"] = [{"full_code": "07008011", "code": "0700_8011", "description": "AMS A Slot 2 filament has run out."}]
    if step == 4:
        s.update(state="FINISH", progress=100.0, remaining_time=0)
    return s


class H(BaseHTTPRequestHandler):
    def send(self, obj, code=200):
        body = json.dumps(obj).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        global step
        p = self.path.split("?")[0]
        if p.startswith("/_step/"):
            step = int(p.rsplit("/", 1)[1])
            return self.send({"step": step})
        if p == "/api/v1/printers/":
            return self.send([{"id": 1, "name": "P1S"}])
        if p == "/api/v1/printers/1/status":
            return self.send(status())
        self.send({"detail": "no camera in the fake"}, 404)

    def do_POST(self):
        if self.path.startswith("/api/v1/printers/camera/stream-token"):
            return self.send({"token": "fake"})
        self.send({})

    def log_message(self, *a):
        pass


if __name__ == "__main__":
    ThreadingHTTPServer(("127.0.0.1", int(sys.argv[1]) if len(sys.argv) > 1 else 8790), H).serve_forever()
