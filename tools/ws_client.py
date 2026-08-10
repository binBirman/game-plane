#!/usr/bin/env python3
"""Minimal WebSocket client for smoke tests. No external deps."""

import base64
import os
import socket
import sys


def handshake(host: str, port: int, path: str):
    """Perform RFC 6455 client handshake. Returns (sock, key)."""
    sock = socket.create_connection((host, port), timeout=5)
    key = base64.b64encode(os.urandom(16)).decode()
    req = (
        f"GET {path} HTTP/1.1\r\n"
        f"Host: {host}:{port}\r\n"
        f"Upgrade: websocket\r\n"
        f"Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\n"
        f"Sec-WebSocket-Version: 13\r\n\r\n"
    )
    sock.sendall(req.encode())
    buf = b""
    while b"\r\n\r\n" not in buf:
        chunk = sock.recv(1024)
        if not chunk:
            raise RuntimeError("connection closed during handshake")
        buf += chunk
    if b" 101 " not in buf.split(b"\r\n", 1)[0]:
        raise RuntimeError(f"handshake failed: {buf[:200]!r}")
    return sock


def send_text(sock, msg: str):
    """Send a masked text frame (client->server must mask)."""
    data = msg.encode("utf-8")
    mask = os.urandom(4)
    masked = bytes(d ^ mask[i % 4] for i, d in enumerate(data))
    header = bytes([0x81, 0x80 | len(data)])
    sock.sendall(header + mask + masked)


def recv_text(sock) -> str | None:
    """Receive a text frame. Returns None on close or non-text."""
    def recvn(n):
        buf = b""
        while len(buf) < n:
            chunk = sock.recv(n - len(buf))
            if not chunk:
                raise RuntimeError("connection closed mid-frame")
            buf += chunk
        return buf

    hdr = recvn(2)
    opcode = hdr[0] & 0x0F
    if opcode == 0x8:
        return None  # close
    if opcode == 0x9:
        return None  # ping, ignore (would normally pong)
    masked = hdr[1] & 0x80
    length = hdr[1] & 0x7F
    if length == 126:
        length = int.from_bytes(recvn(2), "big")
    elif length == 127:
        length = int.from_bytes(recvn(8), "big")
    payload = recvn(length)
    if masked:
        mask = recvn(4)
        payload = bytes(d ^ mask[i % 4] for i, d in enumerate(payload))
    return payload.decode("utf-8")