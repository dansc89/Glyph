#!/usr/bin/env python3
"""Behavioral isolation regressions; never launch Glyph or use a desktop display."""
import ctypes
import ctypes.util
import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('editing_qa', Path(__file__).with_name('editing-qa.py'))
assert spec is not None and spec.loader is not None
qa = importlib.util.module_from_spec(spec)
spec.loader.exec_module(qa)
REAL_POPEN = subprocess.Popen
XVFB = shutil.which('Xvfb')
XLIB = ctypes.util.find_library('X11')


def real_xvfb(cmd, **kwargs):
    if cmd[0] != 'Xvfb':
        raise AssertionError(f'unexpected process: {cmd}')
    assert XVFB is not None
    return REAL_POPEN([XVFB, *cmd[1:]], **kwargs)


def assert_display_connects(test, display):
    # Explicit display only: never consult DISPLAY or connect to the desktop.
    assert XLIB is not None, 'libX11 is required for allocated-display connection coverage'
    xlib = ctypes.CDLL(XLIB)
    xlib.XOpenDisplay.argtypes = [ctypes.c_char_p]
    xlib.XOpenDisplay.restype = ctypes.c_void_p
    xlib.XDisplayWidth.argtypes = [ctypes.c_void_p, ctypes.c_int]
    xlib.XDisplayWidth.restype = ctypes.c_int
    xlib.XCloseDisplay.argtypes = [ctypes.c_void_p]
    connection = xlib.XOpenDisplay(display.encode('ascii'))
    test.assertTrue(connection, f'allocated server {display} accepts X11 clients')
    try:
        test.assertEqual(xlib.XDisplayWidth(connection, 0), 1600)
    finally:
        xlib.XCloseDisplay(connection)


class AppLaunchIntercepted(Exception):
    pass


class IsolationTests(unittest.TestCase):
    def test_legacy_viewer_runner_uses_owned_display_instead_of_fixed_server(self):
        source = Path(__file__).with_name('performance-qa.py').read_text()
        self.assertIn('isolation.PrivateXvfb()', source)
        self.assertNotIn(':79', source)

    def test_failed_owned_server_never_probes_or_launches(self):
        # A real exited process models Xvfb failing while another display responds.
        server = REAL_POPEN([sys.executable, '-c', 'raise SystemExit(1)'])
        server.wait(timeout=5)
        interactions = []

        def popen(cmd, **kwargs):
            if cmd[0] == 'Xvfb':
                return server
            interactions.append(('app', cmd))
            raise AppLaunchIntercepted()

        def run(cmd, **kwargs):
            interactions.append(('command', cmd))
            return subprocess.CompletedProcess(cmd, 0, stdout='', stderr='')

        with tempfile.TemporaryDirectory(prefix='glyph-isolation-test-') as out:
            with mock.patch.object(sys, 'argv', ['editing-qa.py', '--binary', '/never-launch-glyph', '--out', out]), mock.patch.object(qa.subprocess, 'Popen', side_effect=popen), mock.patch.object(qa.subprocess, 'run', side_effect=run):
                try:
                    qa.main()
                except (RuntimeError, AppLaunchIntercepted):
                    pass
        self.assertEqual(interactions, [], 'failed owned Xvfb must not probe another display or launch an app')

    @unittest.skipUnless(XVFB, 'Xvfb must be on PATH for real startup failure coverage')
    def test_actual_xvfb_startup_failure_blocks_all_interaction(self):
        servers = []

        def fail_startup(cmd, **kwargs):
            assert XVFB is not None
            self.assertEqual(cmd[0], 'Xvfb', 'must not start an app after Xvfb failure')
            server = REAL_POPEN([XVFB, *cmd[1:], '-invalid-isolation-test-option'], **kwargs)
            servers.append(server)
            return server

        with tempfile.TemporaryDirectory(prefix='glyph-isolation-test-') as out:
            with mock.patch.object(sys, 'argv', ['editing-qa.py', '--binary', '/never-launch-glyph', '--out', out]), mock.patch.object(qa.subprocess, 'Popen', side_effect=fail_startup), mock.patch.object(qa.subprocess, 'run') as interaction:
                with self.assertRaisesRegex(RuntimeError, 'owned Xvfb'):
                    qa.main()
                interaction.assert_not_called()
        self.assertEqual(len(servers), 1)
        self.assertNotEqual(servers[0].poll(), None)
        self.assertNotEqual(servers[0].returncode, 0)

    def test_allocation_timeout_terminates_owned_process(self):
        servers = []

        def stall(cmd, **kwargs):
            server = REAL_POPEN([sys.executable, '-c', 'import time; time.sleep(30)'], **kwargs)
            servers.append(server)
            return server

        with mock.patch.object(qa.subprocess, 'Popen', side_effect=stall):
            with self.assertRaisesRegex(RuntimeError, 'before timeout'):
                qa.PrivateXvfb(timeout=.05)
        self.assertIsNotNone(servers[0].poll())

    def test_invalid_allocation_terminates_owned_process(self):
        servers = []

        def invalid(cmd, **kwargs):
            fd = cmd[cmd.index('-displayfd') + 1]
            script = 'import os, sys, time; os.write(int(sys.argv[1]), b"not-a-display\\n"); time.sleep(30)'
            server = REAL_POPEN([sys.executable, '-c', script, fd], **kwargs)
            servers.append(server)
            return server

        with mock.patch.object(qa.subprocess, 'Popen', side_effect=invalid):
            with self.assertRaisesRegex(RuntimeError, 'invalid Xvfb display allocation'):
                qa.PrivateXvfb(timeout=1)
        self.assertIsNotNone(servers[0].poll())

    @unittest.skipUnless(XVFB, 'Xvfb must be on PATH for real allocation coverage')
    def test_two_real_servers_allocate_distinct_connectable_displays(self):
        servers = []
        try:
            with mock.patch.object(qa.subprocess, 'Popen', side_effect=real_xvfb):
                first = qa.PrivateXvfb()
                servers.append(first)
                # Simulate an existing display in the calling environment.
                with mock.patch.dict(os.environ, DISPLAY=first.env['DISPLAY'], WAYLAND_DISPLAY='do-not-use-wayland'):
                    second = qa.PrivateXvfb()
                servers.append(second)
            self.assertNotEqual(first.env['DISPLAY'], second.env['DISPLAY'])
            for owned in servers:
                owned.ensure_alive()
                self.assertEqual(owned.env['WAYLAND_DISPLAY'], '')
                self.assertEqual(owned.env['WINIT_UNIX_BACKEND'], 'x11')
                assert_display_connects(self, owned.env['DISPLAY'])
        finally:
            for owned in servers:
                owned.close()
                self.assertIsNotNone(owned.server.poll())

    @unittest.skipUnless(XVFB, 'Xvfb must be on PATH for real allocation coverage')
    def test_main_uses_allocated_display_for_app_input_and_screenshots(self):
        servers = []
        apps = []
        commands = []

        def popen(cmd, **kwargs):
            if cmd[0] == 'Xvfb':
                server = real_xvfb(cmd, **kwargs)
                servers.append(server)
                return server
            self.assertIsNone(servers[0].poll())
            self.assertEqual(kwargs['env']['WAYLAND_DISPLAY'], '')
            self.assertEqual(kwargs['env']['WINIT_UNIX_BACKEND'], 'x11')
            assert_display_connects(self, kwargs['env']['DISPLAY'])
            app = REAL_POPEN([sys.executable, '-c', 'import time; time.sleep(30)'], **kwargs)
            apps.append((app, kwargs['env']['DISPLAY']))
            return app

        def run(cmd, **kwargs):
            # No input is actually delivered, even on the owned display.
            self.assertIsNone(servers[0].poll())
            self.assertEqual(kwargs['env']['DISPLAY'], apps[0][1])
            commands.append(cmd)
            stdout = ''
            if cmd[0] == 'xdotool' and cmd[1] == 'search':
                stdout = '123\n'
            elif cmd[0] == 'ffmpeg':
                self.assertEqual(cmd[cmd.index('-i') + 1], apps[0][1])
                qa.Image.new('RGB', (8, 8), 'white').save(cmd[-1])
            elif cmd[0] == 'tesseract':
                stdout = 'left\ttop\twidth\theight\ttext\n0\t0\t4\t4\tDocument\n0\t0\t4\t4\tSave\n'
            return subprocess.CompletedProcess(cmd, 0, stdout=stdout, stderr='')

        try:
            with tempfile.TemporaryDirectory(prefix='glyph-isolation-test-') as out:
                with mock.patch.object(sys, 'argv', ['editing-qa.py', '--binary', '/never-launch-glyph', '--out', out, '--document-menu-only']), mock.patch.object(qa.subprocess, 'Popen', side_effect=popen), mock.patch.object(qa.subprocess, 'run', side_effect=run), mock.patch('builtins.print'):
                    qa.main()
                self.assertTrue(any(cmd[0] == 'ffmpeg' for cmd in commands))
                self.assertTrue(any(cmd[0] == 'xdotool' and 'click' in cmd for cmd in commands))
            self.assertIsNotNone(servers[0].poll())
            self.assertIsNotNone(apps[0][0].poll())
        finally:
            for process in servers + [app for app, _ in apps]:
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=5)

    @unittest.skipUnless(XVFB, 'Xvfb must be on PATH for real allocation coverage')
    def test_server_death_before_app_start_blocks_launch(self):
        factory = qa.PrivateXvfb
        owned = []

        def dead_display():
            with mock.patch.object(qa.subprocess, 'Popen', side_effect=real_xvfb):
                display = factory()
            display.close()
            owned.append(display)
            return display

        with tempfile.TemporaryDirectory(prefix='glyph-isolation-test-') as out:
            with mock.patch.object(sys, 'argv', ['editing-qa.py', '--binary', '/never-launch-glyph', '--out', out]), mock.patch.object(qa, 'PrivateXvfb', side_effect=dead_display), mock.patch.object(qa.subprocess, 'Popen') as app_start, mock.patch.object(qa.subprocess, 'run') as interaction:
                with self.assertRaisesRegex(RuntimeError, 'owned Xvfb is not running'):
                    qa.main()
                app_start.assert_not_called()
                interaction.assert_not_called()
        self.assertIsNotNone(owned[0].server.poll())

    @unittest.skipUnless(XVFB, 'Xvfb must be on PATH for real allocation coverage')
    def test_server_death_before_input_blocks_commands(self):
        servers = []
        apps = []

        def popen(cmd, **kwargs):
            if cmd[0] == 'Xvfb':
                server = real_xvfb(cmd, **kwargs)
                servers.append(server)
                return server
            self.assertIsNone(servers[0].poll())
            app = REAL_POPEN([sys.executable, '-c', 'import time; time.sleep(30)'], **kwargs)
            apps.append(app)
            servers[0].terminate()
            servers[0].wait(timeout=5)
            return app

        try:
            with tempfile.TemporaryDirectory(prefix='glyph-isolation-test-') as out:
                with mock.patch.object(sys, 'argv', ['editing-qa.py', '--binary', '/never-launch-glyph', '--out', out]), mock.patch.object(qa.subprocess, 'Popen', side_effect=popen), mock.patch.object(qa.subprocess, 'run') as interaction:
                    with self.assertRaisesRegex(RuntimeError, 'owned Xvfb is not running'):
                        qa.main()
                    interaction.assert_not_called()
            self.assertIsNotNone(apps[0].poll())
        finally:
            for process in servers + apps:
                if process.poll() is None:
                    process.terminate()
                    process.wait(timeout=5)


if __name__ == '__main__':
    unittest.main(verbosity=2)
