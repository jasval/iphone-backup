#!/usr/bin/env python3
"""
Pairing-file upload server — stdlib only, port 8765.

GET  /        → serves /opt/iphone-backup/www/upload.html
GET  /upload  → same
POST /upload  → saves .plist files to /var/lib/lockdown/, returns JSON
"""
import cgi
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

LOCKDOWN_DIR = "/var/lib/lockdown"
UPLOAD_HTML  = "/opt/iphone-backup/www/upload.html"
PORT         = 8765
PLIST_MAGIC  = (b"<?xml", b"bplist")


def validate_plist(filename: str, data: bytes) -> "str | None":
    """Return an error string if invalid, None if OK."""
    if not filename.lower().endswith(".plist"):
        return "filename must end in .plist"
    if not any(data.startswith(m) for m in PLIST_MAGIC):
        return "not a valid plist (bad magic bytes)"
    return None


class Handler(BaseHTTPRequestHandler):
    def log_message(self, fmt, *args):
        print(f"[upload_server] {self.address_string()} - {fmt % args}", flush=True)

    def send_json(self, code: int, payload: dict):
        body = json.dumps(payload).encode()
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def serve_html(self):
        try:
            data = open(UPLOAD_HTML, "rb").read()
        except OSError:
            self.send_response(500)
            self.end_headers()
            self.wfile.write(b"upload.html not found")
            return
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        path = self.path.split("?")[0].rstrip("/")
        if path in ("", "/", "/upload", "/pairing"):
            self.serve_html()
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        path = self.path.split("?")[0].rstrip("/")
        if path not in ("/upload", "/pairing"):
            self.send_response(404)
            self.end_headers()
            return

        form = cgi.FieldStorage(
            fp=self.rfile,
            headers=self.headers,
            environ={
                "REQUEST_METHOD": "POST",
                "CONTENT_TYPE": self.headers.get("Content-Type", ""),
                "CONTENT_LENGTH": self.headers.get("Content-Length", "0"),
            },
        )

        if "file" not in form:
            self.send_json(400, {"ok": False, "error": "no file field in form"})
            return

        item = form["file"]
        filename = item.filename or "unknown.plist"
        data = item.file.read()

        err = validate_plist(filename, data)
        if err:
            print(f"[upload_server] rejected '{filename}': {err}", flush=True)
            self.send_json(400, {"ok": False, "error": err})
            return

        os.makedirs(LOCKDOWN_DIR, exist_ok=True)
        dest = os.path.join(LOCKDOWN_DIR, os.path.basename(filename))
        with open(dest, "wb") as fh:
            fh.write(data)
        print(f"[upload_server] saved {dest} ({len(data)} bytes)", flush=True)
        self.send_json(200, {"ok": True, "saved": os.path.basename(filename)})


if __name__ == "__main__":
    server = HTTPServer(("0.0.0.0", PORT), Handler)
    print(f"[upload_server] listening on :{PORT}", flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        sys.exit(0)
