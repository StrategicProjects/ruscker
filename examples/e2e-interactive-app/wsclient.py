# Minimal stdlib WebSocket client: connect through a path,
# send one text frame, read one echo frame. No deps.
import socket, base64, os, sys, struct

def ws_echo(host, port, path, payload):
    key = base64.b64encode(os.urandom(16)).decode()
    req = (
        f"GET {path} HTTP/1.1\r\n"
        f"Host: {host}:{port}\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\n"
        "Sec-WebSocket-Version: 13\r\n"
        "\r\n"
    )
    s = socket.create_connection((host, port), timeout=10)
    s.sendall(req.encode())
    # Read handshake response headers
    buf = b""
    while b"\r\n\r\n" not in buf:
        chunk = s.recv(4096)
        if not chunk:
            raise RuntimeError("connection closed during handshake")
        buf += chunk
    head = buf.split(b"\r\n\r\n", 1)[0].decode(errors="replace")
    status = head.splitlines()[0]
    if "101" not in status:
        print("HANDSHAKE FAILED:", status)
        print(head)
        return None
    # Send a masked text frame (client frames MUST be masked)
    data = payload.encode()
    mask = os.urandom(4)
    masked = bytes(b ^ mask[i % 4] for i, b in enumerate(data))
    frame = bytes([0x81, 0x80 | len(data)]) + mask + masked  # FIN+text, masked, <126 len
    s.sendall(frame)
    # Read one server frame (server->client is unmasked)
    hdr = s.recv(2)
    if len(hdr) < 2:
        raise RuntimeError("short frame header")
    ln = hdr[1] & 0x7F
    if ln == 126:
        ln = struct.unpack(">H", s.recv(2))[0]
    elif ln == 127:
        ln = struct.unpack(">Q", s.recv(8))[0]
    body = b""
    while len(body) < ln:
        body += s.recv(ln - len(body))
    s.close()
    return body.decode(errors="replace")

if __name__ == "__main__":
    host, port, path, payload = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4]
    result = ws_echo(host, port, path, payload)
    print("RECV:", result)
