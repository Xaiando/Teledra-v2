"""Real loopback HTTP checks for the isolated practice dashboard."""
import http.client
import json
from pathlib import Path
import tempfile
import threading
import unittest

from flycomposer.buzzart.autonomy_dashboard import DashboardServer, MAX_STATUS_BYTES, read_history, read_status


class AutonomyDashboardTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='buzzart-dashboard-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.run = self.root / 'run'
        self.run.mkdir()
        (self.root / 'web').mkdir()
        (self.root / 'web/autonomy.html').write_text('<html>Practice dashboard</html>', encoding='utf-8')
        (self.run / 'status.json').write_text(json.dumps({'status': 'training', 'generation': 8, 'candidates': []}))
        (self.run / 'listening/candidate-1').mkdir(parents=True)
        self.audio = bytes(range(256)) * 2
        (self.run / 'listening/candidate-1/audio.wav').write_bytes(self.audio)
        reference = self.root / 'reports/longform-development/final-dusk'
        reference.mkdir(parents=True)
        (reference / 'audio.wav').write_bytes(b'reference audio')
        self.server = DashboardServer(self.run, port=0, root=self.root)
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={'poll_interval': .02}, daemon=True)
        self.thread.start()
        self.addCleanup(self.close_server)

    def close_server(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)

    def request(self, method, path, body=None, headers=None):
        connection = http.client.HTTPConnection('127.0.0.1', self.server.server_port, timeout=5)
        try:
            connection.request(method, path, body=body, headers=headers or {})
            response = connection.getresponse()
            return response.status, dict(response.getheaders()), response.read()
        finally:
            connection.close()

    def control(self, action, origin=True):
        headers = {'Content-Type': 'application/json'}
        if origin:
            headers['Origin'] = f'http://127.0.0.1:{self.server.server_port}' if origin is True else origin
        return self.request('POST', '/api/control', json.dumps({'action': action}), headers)

    def test_status_is_live_and_references_are_fixed(self):
        status, _, body = self.request('GET', '/api/status')
        self.assertEqual(status, 200)
        data = json.loads(body)
        self.assertEqual(data['generation'], 8)
        self.assertEqual(data['dashboard']['references'][0]['audio_url'], '/reference/dusk/audio.wav')
        self.assertTrue(data['dashboard']['references'][0]['available'])
        self.assertFalse(data['dashboard']['references'][1]['available'])
        (self.run / 'status.json').write_text('{"status":"training","generation":9}')
        self.assertEqual(json.loads(self.request('GET', '/api/status')[2])['generation'], 9)
        self.assertEqual(self.request('GET', '/reference/dusk/audio.wav')[2], b'reference audio')
        self.assertFalse((self.root / 'library').exists())

    def test_audio_ranges_and_head(self):
        status, headers, body = self.request('GET', '/audio/candidate-1/audio.wav', headers={'Range': 'bytes=10-19'})
        self.assertEqual((status, body), (206, self.audio[10:20]))
        self.assertEqual(headers['Content-Range'], 'bytes 10-19/512')
        self.assertEqual(self.request('GET', '/audio/candidate-1/audio.wav', headers={'Range': 'bytes=-4'})[2], self.audio[-4:])
        self.assertEqual(self.request('GET', '/audio/candidate-1/audio.wav', headers={'Range': 'bytes=999-'})[0], 416)
        self.assertEqual(self.request('GET', '/audio/candidate-1/audio.wav', headers={'Range': 'bytes=1-2,4-8'})[0], 416)
        status, headers, body = self.request('HEAD', '/audio/candidate-1/audio.wav')
        self.assertEqual((status, len(body), headers['Content-Length']), (200, 0, '512'))

    def test_paths_and_non_loopback_host_are_refused(self):
        for path in ('/audio/../status.json', '/audio/%2e%2e/audio.wav', '/audio/candidate-1/%2e%2e/status.json',
                     '/audio/candidate-1/score.json', '/audio/candidate-1/lead.wav', '/reference/other/audio.wav'):
            self.assertEqual(self.request('GET', path)[0], 404, path)
        self.assertEqual(self.request('GET', '/api/status', headers={'Host': f'evil.example:{self.server.server_port}'})[0], 403)

    def test_controls_require_same_origin_and_write_only_markers(self):
        original = (self.run / 'status.json').read_bytes()
        self.assertEqual(self.control('pause', origin=False)[0], 403)
        self.assertEqual(self.control('pause', origin='https://evil.example')[0], 403)
        self.assertEqual(self.control('pause', origin='http://127.0.0.1:1')[0], 403)
        self.assertFalse((self.run / 'PAUSE').exists())
        self.assertEqual(self.control('pause')[0], 200)
        self.assertEqual(json.loads((self.run / 'PAUSE').read_text())['action'], 'pause')
        self.assertEqual(self.control('resume')[0], 200)
        self.assertFalse((self.run / 'PAUSE').exists())
        self.assertEqual(self.control('stop')[0], 200)
        self.assertTrue((self.run / 'STOP').exists())
        self.assertEqual(self.control('resume')[0], 409)
        self.assertEqual((self.run / 'status.json').read_bytes(), original)
        self.assertEqual(list(self.run.glob('.control-*')), [])

    def test_status_and_history_reads_are_bounded(self):
        (self.run / 'status.json').write_bytes(b' ' * (MAX_STATUS_BYTES + 1))
        with self.assertRaisesRegex(ValueError, 'read limit'):
            read_status(self.run)
        self.assertEqual(self.request('GET', '/api/status')[0], 422)
        rows = ''.join(json.dumps({'generation': n}) + '\n' for n in range(130))
        (self.run / 'history.jsonl').write_text('x' * (2 * 1024 * 1024) + '\n' + rows + '{"partial":', encoding='utf-8')
        records = read_history(self.run)
        self.assertEqual(len(records), 100)
        self.assertEqual((records[0]['generation'], records[-1]['generation']), (30, 129))

    def test_symlink_cannot_escape_run_or_delete_status(self):
        outside = self.root / 'outside'
        outside.mkdir()
        (outside / 'audio.wav').write_bytes(b'private outside audio')
        try:
            (self.run / 'listening/escape').symlink_to(outside, target_is_directory=True)
            (self.run / 'PAUSE').symlink_to(self.run / 'status.json')
        except OSError as error:
            self.skipTest(f'Local environment cannot create test symlinks: {error}')
        self.assertEqual(self.request('GET', '/audio/escape/audio.wav')[0], 422)
        self.assertEqual(self.control('resume')[0], 400)
        self.assertTrue((self.run / 'status.json').is_file())


if __name__ == '__main__':
    unittest.main()
