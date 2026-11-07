import fcntl
import os
import pty
import select
import struct
import subprocess
import sys
import termios
import time


def check_terminal(binary, scenario):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 30, 100, 0, 0))
    # macOS revokes the slave when its controlling session exits; the master stays readable.
    original = termios.tcgetattr(master)
    reader, writer = os.pipe()
    if scenario == "init_error":
        os.close(reader)
        reader = None
    process = None

    def set_terminal():
        os.setsid()
        fcntl.ioctl(0, termios.TIOCSCTTY, 0)

    try:
        process = subprocess.Popen(
            [binary, "tui", "foo"],
            stdin=slave,
            stdout=writer,
            stderr=subprocess.PIPE,
            preexec_fn=set_terminal,
            env={
                **os.environ,
                "GITHUB_TOKEN": "test-token",
                "GH_CHK_API_BASE_URL": "http://127.0.0.1:1",
                "GH_CHK_GRAPHQL_URL": "http://127.0.0.1:1/graphql",
            },
        )
        os.close(writer)
        writer = None
        output = b""
        sent_key = False
        deadline = time.monotonic() + 10
        while reader is not None:
            remaining = deadline - time.monotonic()
            assert remaining > 0, f"{scenario}: terminal output timed out"
            ready, _, _ = select.select([reader], [], [], remaining)
            assert ready, f"{scenario}: terminal output timed out"
            chunk = os.read(reader, 65536)
            if not chunk:
                break
            output += chunk
            if not sent_key and b"Help" in output:
                assert termios.tcgetattr(master) != original, "TUI did not enter raw mode"
                sent_key = True
                if scenario == "draw_error":
                    os.close(reader)
                    reader = None
                    # Changing the screen forces another write to the closed pipe.
                    os.write(master, b"s")
                else:
                    os.write(master, b"q")

        _, stderr = process.communicate(timeout=10)
        assert termios.tcgetattr(master) == original, f"{scenario}: raw mode was not restored"
        if scenario == "quit":
            assert process.returncode == 0, stderr.decode()
            assert b"\x1b[?1049l" in output, "alternate screen was not left"
            assert b"\x1b[?1000l" in output, "mouse capture was not disabled"
            assert b"\x1b[?25h" in output, "cursor was not shown"
        else:
            assert process.returncode > 0, f"{scenario}: expected an error exit"
            assert b"TUI error:" in stderr and b"Broken pipe" in stderr, stderr.decode()
        if scenario != "init_error":
            assert sent_key, f"{scenario}: first frame was not rendered"
    finally:
        if process is not None:
            if process.poll() is None:
                process.kill()
            process.communicate()
        for fd in (reader, writer, slave, master):
            if fd is not None:
                os.close(fd)


for scenario in ("quit", "init_error", "draw_error"):
    check_terminal(sys.argv[1], scenario)
