"""Loopback-only UI; no court process, model service, or automatic playback."""
from __future__ import annotations

from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
import json
import mimetypes
from pathlib import Path
import re
from urllib.parse import urlsplit


def make_server(studio, port=8789):
    class Handler(BaseHTTPRequestHandler):
        def log_message(self,*args):pass
        def send_json(self,value,status=200):
            body=json.dumps(value,ensure_ascii=False).encode()
            self.send_response(status);self.send_header('Content-Type','application/json; charset=utf-8')
            self.send_header('Cache-Control','no-store');self.send_header('Content-Length',str(len(body)))
            self.end_headers();self.wfile.write(body)
        def do_GET(self):
            path=urlsplit(self.path).path
            if path=='/api/state':return self.send_json(studio.state())
            if path in ('/','/app.js','/styles.css'):
                target=studio.root/'web'/('index.html' if path=='/' else path[1:])
            elif match:=re.fullmatch(r'/audio/([0-9a-f]{16})/(audio|lead|answer|harmony|bass|texture)\.wav',path):
                if not any(t['id']==match[1] for t in studio.library['tracks']):return self.send_error(404)
                target=studio.root/'library'/match[1]/f'{match[2]}.wav'
            else:return self.send_error(404)
            if not target.is_file():return self.send_error(404)
            size=target.stat().st_size;start=0;end=size-1
            if requested:=self.headers.get('Range'):
                match=re.fullmatch(r'bytes=(\d+)-(\d*)',requested)
                if not match:return self.send_error(416)
                start=int(match[1]);end=min(end,int(match[2]) if match[2] else end)
                if start>end:return self.send_error(416)
                self.send_response(206);self.send_header('Content-Range',f'bytes {start}-{end}/{size}')
            else:self.send_response(200)
            self.send_header('Content-Type',mimetypes.guess_type(target)[0] or 'application/octet-stream')
            self.send_header('Content-Length',str(end-start+1));self.send_header('Accept-Ranges','bytes')
            self.send_header('Cache-Control','no-cache');self.end_headers()
            try:
                with target.open('rb') as stream:
                    stream.seek(start);remaining=end-start+1
                    while remaining:
                        chunk=stream.read(min(65536,remaining))
                        if not chunk:break
                        self.wfile.write(chunk);remaining-=len(chunk)
            except (BrokenPipeError,ConnectionResetError):pass
        def do_POST(self):
            origin=self.headers.get('Origin')
            if origin and origin not in (f'http://127.0.0.1:{self.server.server_port}',f'http://localhost:{self.server.server_port}'):
                return self.send_error(403)
            if self.headers.get_content_type()!='application/json':return self.send_error(415)
            try:
                length=int(self.headers.get('Content-Length','0'))
                if not 0<length<=8192:raise ValueError('Request body is too large or empty')
                data=json.loads(self.rfile.read(length))
                if not isinstance(data,dict):raise ValueError('JSON object required')
                if self.path=='/api/perform':
                    studio.start_performance(data.get('seed',2026),data.get('seconds',75),retry_id=data.get('retry_id'),source_id=data.get('source_id'))
                    return self.send_json({'started':True},202)
                if self.path=='/api/organist-study':
                    studio.start_organist_study(data.get('source_id'),data.get('feedback'),data.get('seconds',75))
                    return self.send_json({'started':True},202)
                if self.path=='/api/revise':
                    studio.start_revision(data.get('track_id'),data.get('revision','connected_phrases_v1'))
                    return self.send_json({'started':True},202)
                if self.path in ('/api/compose','/api/develop'):
                    seed=data.get('seed');ensemble=data.get('ensemble')
                    if type(seed)is not int or not 0<=seed<=2**31-1:raise ValueError('Invalid seed')
                    from .composition import ENSEMBLES
                    if ensemble is not None and ensemble not in ENSEMBLES:raise ValueError('Invalid ensemble')
                    if self.path=='/api/develop':studio.start_development(seed,ensemble,data.get('voices'))
                    else:studio.start(seed,ensemble or studio.state()['preferences']['preferred_ensemble'],bool(data.get('blind',True)),data.get('voices'))
                    return self.send_json({'started':True},202)
                if self.path=='/api/feedback':studio.feedback(data)
                elif self.path=='/api/comparison-feedback':studio.comparison_action(data)
                elif self.path=='/api/reveal':studio.comparison_action(data,reveal=True)
                else:return self.send_error(404)
                self.send_json({'ok':True})
            except (ValueError,KeyError,TypeError) as error:self.send_json({'error':str(error)},400)
    return ThreadingHTTPServer(('127.0.0.1',port),Handler)
