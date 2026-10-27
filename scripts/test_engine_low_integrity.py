"""Windows-only live IPC regression, using a private engine and synthetic data.

Run after building: python scripts/test_engine_low_integrity.py --engine <exe>
Never registers a TIP or touches the installed engine/configuration.
"""

import argparse
import ctypes as C
from ctypes import wintypes as W
import os
from pathlib import Path
import secrets
import struct
import subprocess
import time
import uuid


class Blob(C.Structure):
    _fields_ = [("size", W.DWORD), ("data", C.c_void_p)]


class SidAttributes(C.Structure):
    _fields_ = [("sid", C.c_void_p), ("attributes", W.DWORD)]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--engine", type=Path, required=True)
    args = parser.parse_args()
    engine = args.engine.resolve(strict=True)
    kernel = C.WinDLL("kernel32", use_last_error=True)
    security = C.WinDLL("advapi32", use_last_error=True)
    crypt = C.WinDLL("crypt32", use_last_error=True)

    def api(dll, name, result, *arguments):
        function = getattr(dll, name)
        function.restype, function.argtypes = result, arguments
        return function

    close = api(kernel, "CloseHandle", W.BOOL, W.HANDLE)
    free = api(kernel, "LocalFree", C.c_void_p, C.c_void_p)
    current = api(kernel, "GetCurrentProcess", W.HANDLE)
    open_token = api(security, "OpenProcessToken", W.BOOL, W.HANDLE, W.DWORD, C.POINTER(W.HANDLE))
    duplicate = api(security, "DuplicateTokenEx", W.BOOL, W.HANDLE, W.DWORD, C.c_void_p, C.c_int, C.c_int, C.POINTER(W.HANDLE))
    sid_from_string = api(security, "ConvertStringSidToSidW", W.BOOL, W.LPCWSTR, C.POINTER(C.c_void_p))
    set_information = api(security, "SetTokenInformation", W.BOOL, W.HANDLE, C.c_int, C.c_void_p, W.DWORD)
    set_thread = api(security, "SetThreadToken", W.BOOL, C.c_void_p, W.HANDLE)
    revert = api(security, "RevertToSelf", W.BOOL)
    create_file = api(kernel, "CreateFileW", W.HANDLE, W.LPCWSTR, W.DWORD, W.DWORD, C.c_void_p, W.DWORD, W.DWORD, W.HANDLE)
    read = api(kernel, "ReadFile", W.BOOL, W.HANDLE, C.c_void_p, W.DWORD, C.POINTER(W.DWORD), C.c_void_p)
    write = api(kernel, "WriteFile", W.BOOL, W.HANDLE, C.c_void_p, W.DWORD, C.POINTER(W.DWORD), C.c_void_p)
    protect = api(crypt, "CryptProtectData", W.BOOL, C.POINTER(Blob), C.c_void_p, C.c_void_p, C.c_void_p, C.c_void_p, W.DWORD, C.POINTER(Blob))

    def require(result):
        if not result:
            raise C.WinError(C.get_last_error())

    def connect(pipe_name, timeout=5):
        deadline = time.monotonic() + timeout
        while True:
            handle = create_file(pipe_name, 0xC0000000, 0, None, 3, 0x110000, None)
            if handle != W.HANDLE(-1).value:
                return handle
            error = C.get_last_error()
            if error not in (2, 231) or time.monotonic() >= deadline:
                raise C.WinError(error)
            time.sleep(0.01)

    run_id = uuid.uuid4().hex
    root = Path(__file__).resolve().parents[1] / "dist" / ("win11-ipc-live-" + run_id)
    data = root / "kaixin"
    data.mkdir(parents=True)
    token = secrets.token_hex(32)
    plain = C.create_string_buffer(token.encode("ascii"))
    blob, encrypted = Blob(len(token), C.cast(plain, C.c_void_p)), Blob()
    require(protect(C.byref(blob), None, None, None, None, 1, C.byref(encrypted)))
    try:
        (data / "engine_capability.dat").write_bytes(b"KXIPC-DPAPI-1\n" + C.string_at(encrypted.data, encrypted.size))
    finally:
        free(encrypted.data)

    original, low, sid = W.HANDLE(), W.HANDLE(), C.c_void_p()
    require(open_token(current(), 0xA, C.byref(original)))  # QUERY | DUPLICATE
    try:
        require(duplicate(original, 0xF01FF, None, 2, 2, C.byref(low)))
    finally:
        close(original)
    try:
        require(sid_from_string("S-1-16-4096", C.byref(sid)))
        try:
            label = SidAttributes(sid, 0x20)
            require(set_information(low, 25, C.byref(label), C.sizeof(label)))
        finally:
            free(sid)

        pipe_name = rf"\\.\pipe\KaixinWin11Live-{run_id}"
        env = dict(os.environ, LOCALAPPDATA=str(root), SRF_ENGINE_PIPE_NAME=pipe_name,
                   SRF_ENGINE_MUTEX_NAME=rf"Local\KaixinWin11Live-{run_id}")
        process = subprocess.Popen([str(engine), "--no-warmup"], env=env,
                                   creationflags=subprocess.CREATE_NO_WINDOW)
        try:
            deadline = time.monotonic() + 15
            while True:
                handle = create_file(pipe_name, 0xC0000000, 0, None, 3, 0x110000, None)
                if handle != W.HANDLE(-1).value:
                    close(handle)
                    break
                if process.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("isolated engine did not start")
                time.sleep(0.05)

            def request(command, payload, low_integrity):
                if low_integrity:
                    require(set_thread(None, low))
                handle = W.HANDLE(-1).value
                try:
                    handle = connect(pipe_name)
                    packet = struct.pack("<IHHI", 0x31504653, 5, command, len(payload)) + payload
                    count = W.DWORD()
                    require(write(handle, packet, len(packet), C.byref(count), None))
                    assert count.value == len(packet)
                finally:
                    if low_integrity:
                        require(revert())

                def receive(size):
                    output = bytearray()
                    while len(output) < size:
                        buffer = C.create_string_buffer(size - len(output))
                        count = W.DWORD()
                        require(read(handle, buffer, len(buffer), C.byref(count), None))
                        if count.value == 0:
                            raise RuntimeError("engine disconnected before response")
                        output.extend(buffer.raw[:count.value])
                    return bytes(output)

                try:
                    magic, version, returned_command, status, size = struct.unpack("<IHH i I", receive(16))
                    assert (magic, version, returned_command) == (0x31504653, 5, command)
                    assert size < 4 * 1024 * 1024
                    return status, receive(size)
                finally:
                    close(handle)

            def field(value):
                encoded = value.encode("utf-16-le")
                return struct.pack("<I", len(encoded) // 2) + encoded

            for low_integrity in (False, True):
                assert request(7, b"", low_integrity)[0] == 0, "identity check accepts same-session desktop host"
                status, _ = request(2, field("0" * 64) + struct.pack("<I", 0) + field("nihao"), low_integrity)
                assert status == -6, "wrong capability must remain denied after lowering pipe integrity"
                deadline = time.monotonic() + 30
                while True:
                    payload = field(token) + struct.pack("<I", 0) + field("nihao") + struct.pack("<Q", 2 if low_integrity else 1)
                    status, response = request(2, payload, low_integrity)
                    if status != -5 or time.monotonic() >= deadline:
                        break
                    time.sleep(0.05)  # Retry a bounded busy response during warmup.
                assert status == 0 and len(response) >= 4 and struct.unpack_from("<I", response)[0] > 0, f"authenticated desktop host receives candidates: status={status} bytes={len(response)}"
                print(f"PASS: {'low' if low_integrity else 'normal'} integrity: health, wrong-token denial, authenticated lookup")
            assert request(9, field(token), False)[0] == 0
            process.wait(timeout=10)
            print("PASS: isolated engine shutdown")
        finally:
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=10)
    finally:
        close(low)


if __name__ == "__main__":
    main()
