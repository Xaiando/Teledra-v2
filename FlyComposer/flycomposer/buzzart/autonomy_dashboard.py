"""Loopback-only status and listening room for one autonomous practice run.

The runner owns status, history and recordings. This server reads those files
and writes only the explicit STOP/PAUSE control markers; it never opens Studio
or changes the main listening library.
"""
from __future__ import annotations

import argparse
import datetime as dt
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import re
from urllib.parse import unquote, urlsplit
from uuid import uuid4

MAX_STATUS_BYTES = 512 * 1024
MAX_HISTORY_BYTES = 1024 * 1024
MAX_CONTROL_BYTES = 4096
SAFE_ID = re.compile(r'[A-Za-z0-9][A-Za-z0-9_-]{0,79}\Z')
LOOPBACK_NAMES = {'127.0.0.1', 'localhost', '::1'}


def _inside(path, parent):
    resolved = path.resolve()
    if not resolved.is_relative_to(parent.resolve()):
        raise ValueError('Path leaves the run directory')
    return resolved


def read_status(run_dir):
    path = _inside(Path(run_dir) / 'status.json', Path(run_dir))
    if not path.exists():
        return {'status': 'waiting', 'phase': 'Waiting for the runner', 'candidates': [],
                'message': 'The practice run has not written its first update yet.'}
    with path.open('rb') as stream:
        raw = stream.read(MAX_STATUS_BYTES + 1)
    if len(raw) > MAX_STATUS_BYTES:
        raise ValueError('Status exceeds the dashboard read limit')
    result = json.loads(raw)
    if not isinstance(result, dict):
        raise ValueError('Status must be a JSON object')
    return result


def read_history(run_dir, limit=100):
    path = _inside(Path(run_dir) / 'history.jsonl', Path(run_dir))
    if not path.exists():
        return []
    with path.open('rb') as stream:
        size = stream.seek(0, 2)
        start = max(0, size - MAX_HISTORY_BYTES)
        stream.seek(start)
        raw = stream.read(MAX_HISTORY_BYTES)
    lines = raw.splitlines()
    if start and lines:
        lines = lines[1:]  # the first line may have begun before our bounded read
    records = []
    for line in reversed(lines):
        try:
            value = json.loads(line)
        except (ValueError, UnicodeDecodeError):
            continue  # an in-progress final JSONL write is not a failed run
        if isinstance(value, dict):
            records.append(value)
        if len(records) >= min(100, max(1, limit)):
            break
    return list(reversed(records))


class DashboardServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, run_dir, port=8791, root=None):
        self.run_dir = Path(run_dir).resolve(strict=True)
        if not self.run_dir.is_dir():
            raise ValueError('Run directory must be a directory')
        self.root = Path(root).resolve() if root else Path(__file__).resolve().parents[2]
        self.html = self.root / 'web/autonomy.html'
        self.references = {context: self.root / 'reports/longform-development' / f'final-{context}' / 'audio.wav'
                           for context in ('dusk', 'overworld')}
        super().__init__(('127.0.0.1', port), DashboardHandler)


class DashboardHandler(BaseHTTPRequestHandler):
    server_version = 'BuzzartPractice/1'

    def log_message(self, fmt, *args):
        pass

    def _headers(self, status, content_type, length=None, extra=None):
        self.send_response(status)
        self.send_header('Content-Type', content_type)
        self.send_header('Cache-Control', 'no-store')
        self.send_header('X-Content-Type-Options', 'nosniff')
        self.send_header('Referrer-Policy', 'no-referrer')
        self.send_header('Content-Security-Policy', "default-src 'self'; script-src 'self' 'unsafe-inline'; style-src 'self' 'unsafe-inline'; media-src 'self'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'")
        if length is not None:
            self.send_header('Content-Length', str(length))
        for key, value in (extra or {}).items():
            self.send_header(key, str(value))
        self.end_headers()

    def _json(self, value, status=200):
        data = json.dumps(value, ensure_ascii=False, allow_nan=False).encode('utf-8')
        self._headers(status, 'application/json; charset=utf-8', len(data))
        if self.command != 'HEAD':
            self.wfile.write(data)

    def _valid_host(self):
        try:
            parsed = urlsplit('http://' + self.headers.get('Host', ''))
            return (parsed.hostname in LOOPBACK_NAMES and parsed.port == self.server.server_port
                    and not parsed.username and not parsed.password and not parsed.path
                    and not parsed.query and not parsed.fragment)
        except ValueError:
            return False

    def _valid_origin(self):
        try:
            parsed = urlsplit(self.headers.get('Origin', ''))
            host = urlsplit('http://' + self.headers.get('Host', ''))
            return (parsed.scheme == 'http' and parsed.hostname in LOOPBACK_NAMES
                    and parsed.hostname == host.hostname and parsed.port == self.server.server_port
                    and not parsed.username and not parsed.password and not parsed.path
                    and not parsed.query and not parsed.fragment)
        except ValueError:
            return False

    def _controls(self):
        return {'stop_requested': (self.server.run_dir / 'STOP').exists(),
                'pause_requested': (self.server.run_dir / 'PAUSE').exists()}

    def _audio_path(self, path):
        match = re.fullmatch(r'/audio/([^/]+)/audio\.wav', path)
        if match and SAFE_ID.fullmatch(match[1]):
            listening = _inside(self.server.run_dir / 'listening', self.server.run_dir)
            return _inside(listening / match[1] / 'audio.wav', listening)
        match = re.fullmatch(r'/reference/(dusk|overworld)/audio\.wav', path)
        if match:
            return _inside(self.server.references[match[1]], self.server.root / 'reports/longform-development')
        return None

    def _serve_audio(self, path):
        with path.open('rb') as stream:
            size = os.fstat(stream.fileno()).st_size
            start, end, code = 0, size - 1, 200
            range_header = self.headers.get('Range')
            if range_header:
                match = re.fullmatch(r'bytes=(\d*)-(\d*)', range_header.strip())
                if not match or not any(match.groups()) or not size:
                    self._headers(416, 'audio/wav', 0, {'Content-Range': f'bytes */{size}'})
                    return
                left, right = match.groups()
                if left:
                    start = int(left)
                    end = min(size - 1, int(right)) if right else size - 1
                else:
                    suffix = int(right)
                    start = max(0, size - suffix)
                if start > end or start >= size:
                    self._headers(416, 'audio/wav', 0, {'Content-Range': f'bytes */{size}'})
                    return
                code = 206
            headers = {'Accept-Ranges': 'bytes'}
            if code == 206:
                headers['Content-Range'] = f'bytes {start}-{end}/{size}'
            self._headers(code, 'audio/wav', max(0, end - start + 1), headers)
            if self.command == 'HEAD':
                return
            stream.seek(start)
            remaining = end - start + 1
            while remaining > 0:
                chunk = stream.read(min(65536, remaining))
                if not chunk:
                    break
                self.wfile.write(chunk)
                remaining -= len(chunk)

    def do_HEAD(self):
        self.do_GET()

    def do_GET(self):
        if not self._valid_host():
            self._json({'error': 'Use this dashboard through its loopback address'}, 403)
            return
        path = unquote(urlsplit(self.path).path)
        try:
            if path in ('/', '/index.html'):
                data = self.server.html.read_bytes()
                self._headers(200, 'text/html; charset=utf-8', len(data))
                if self.command != 'HEAD':
                    self.wfile.write(data)
            elif path == '/api/status':
                status = read_status(self.server.run_dir)
                status['controls'] = self._controls()
                status['dashboard'] = {'references': [
                    {'id': context, 'title': f'Approved reference · {context}',
                     'audio_url': f'/reference/{context}/audio.wav', 'available': path.is_file()}
                    for context, path in self.server.references.items()]}
                self._json(status)
            elif path == '/api/history':
                self._json({'history': read_history(self.server.run_dir)})
            else:
                audio = self._audio_path(path)
                if audio is None or not audio.is_file():
                    self._json({'error': 'Not found'}, 404)
                else:
                    self._serve_audio(audio)
        except (BrokenPipeError, ConnectionResetError):
            pass
        except FileNotFoundError:
            self._json({'error': 'Not found'}, 404)
        except (ValueError, UnicodeDecodeError) as error:
            self._json({'error': str(error)}, 422)
        except OSError:
            self._json({'error': 'The dashboard could not read the requested file'}, 503)

    def do_POST(self):
        if not self._valid_host() or not self._valid_origin():
            self._json({'error': 'Controls require this dashboard\'s same loopback origin'}, 403)
            return
        if urlsplit(self.path).path != '/api/control':
            self._json({'error': 'Not found'}, 404)
            return
        if self.headers.get('Content-Type', '').split(';')[0].strip().lower() != 'application/json':
            self._json({'error': 'Controls require application/json'}, 415)
            return
        try:
            length = int(self.headers.get('Content-Length', '0'))
            if not 0 < length <= MAX_CONTROL_BYTES:
                self._json({'error': 'Control body exceeds the limit or is empty'}, 413)
                return
            request = json.loads(self.rfile.read(length))
            action = request.get('action') if isinstance(request, dict) else None
            if action not in ('pause', 'resume', 'stop'):
                self._json({'error': 'Choose pause, resume or stop'}, 400)
                return
            if action != 'stop' and (self.server.run_dir / 'STOP').exists():
                self._json({'error': 'A stopped run cannot be resumed from this dashboard'}, 409)
                return
            if action == 'resume':
                marker = self.server.run_dir / 'PAUSE'
                if marker.is_symlink():
                    raise ValueError('Control markers cannot be symbolic links')
                marker.unlink(missing_ok=True)
            else:
                marker = self.server.run_dir / ('STOP' if action == 'stop' else 'PAUSE')
                pending = self.server.run_dir / ('.control-' + uuid4().hex + '.tmp')
                pending.write_text(json.dumps({'action': action, 'at': dt.datetime.now(dt.timezone.utc).isoformat()}) + '\n', encoding='utf-8')
                os.replace(pending, marker)
            self._json({'ok': True, 'action': action, 'controls': self._controls(),
                        'message': 'The runner will apply this request at its next safe checkpoint.'})
        except (ValueError, UnicodeDecodeError):
            self._json({'error': 'Invalid control request'}, 400)
        except OSError:
            self._json({'error': 'The control marker could not be written'}, 503)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run-dir', required=True, type=Path)
    parser.add_argument('--port', type=int, default=8791)
    args = parser.parse_args()
    server = DashboardServer(args.run_dir, args.port)
    print(f'Buzzart practice dashboard: http://127.0.0.1:{server.server_port}/', flush=True)
    try:
        server.serve_forever(poll_interval=.5)
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


if __name__ == '__main__':
    main()
