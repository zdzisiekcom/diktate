#!/usr/bin/env python3
"""
Smoke test for diktate daemon IPC communication.
Sends session.start, waits 2 seconds, sends session.stop,
and waits for optional segment.final notification.
"""

import json
import os
import socket
import sys
import time
from pathlib import Path

def get_socket_path():
    """Get the path to the diktate socket."""
    xdg_runtime_dir = os.environ.get('XDG_RUNTIME_DIR', f'/run/user/{os.getuid()}')
    return os.path.join(xdg_runtime_dir, 'diktate.sock')

def connect_to_daemon(socket_path):
    """Connect to the daemon socket."""
    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        sock.connect(socket_path)
        return sock
    except FileNotFoundError:
        print(f"Error: Socket not found at {socket_path}")
        sys.exit(1)
    except ConnectionRefusedError:
        print(f"Error: Connection refused to {socket_path}")
        sys.exit(1)

def send_message(sock, method, msg_id):
    """Send a JSON-RPC message."""
    message = {
        "jsonrpc": "2.0",
        "id": msg_id,
        "method": method
    }
    message_str = json.dumps(message)
    print(f"→ Sent: {message_str}")
    sock.sendall((message_str + '\n').encode('utf-8'))

def read_message(sock, timeout=None):
    """Read a JSON-RPC message from the socket."""
    sock.settimeout(timeout)
    try:
        data = b''
        while True:
            chunk = sock.recv(1)
            if not chunk:
                return None
            data += chunk
            if data.endswith(b'\n'):
                message_str = data.decode('utf-8').strip()
                return json.loads(message_str)
    except socket.timeout:
        return None
    except json.JSONDecodeError as e:
        print(f"Error decoding JSON: {e}")
        return None

def main():
    socket_path = get_socket_path()
    print(f"Connecting to {socket_path}...")

    sock = connect_to_daemon(socket_path)
    print("Connected.\n")

    success = True

    # Send session.start
    print("=== Phase 1: session.start ===")
    send_message(sock, "session.start", 1)
    response1 = read_message(sock, timeout=5)

    if response1 is None:
        print("← Error: No response received for session.start")
        success = False
    else:
        print(f"← Received: {json.dumps(response1)}")
        if "error" in response1:
            print(f"Error in response: {response1['error']}")
            success = False
        elif "result" not in response1:
            print("Error: No 'result' or 'error' field in response")
            success = False

    # Wait 2 seconds
    print("\nWaiting 2 seconds...")
    time.sleep(2)

    # Send session.stop
    print("\n=== Phase 2: session.stop ===")
    send_message(sock, "session.stop", 2)
    response2 = read_message(sock, timeout=5)

    if response2 is None:
        print("← Error: No response received for session.stop")
        success = False
    else:
        print(f"← Received: {json.dumps(response2)}")
        if "error" in response2:
            print(f"Error in response: {response2['error']}")
            success = False
        elif "result" not in response2:
            print("Error: No 'result' or 'error' field in response")
            success = False

    # Wait for optional segment.final notification
    print("\n=== Phase 3: Waiting for segment.final notification (up to 60 seconds) ===")
    start_time = time.time()
    notification_received = False

    while time.time() - start_time < 60:
        remaining = 60 - (time.time() - start_time)
        notification = read_message(sock, timeout=remaining)

        if notification is None:
            break

        print(f"← Received: {json.dumps(notification)}")

        if "method" in notification and notification["method"] == "segment.final":
            notification_received = True
            if "params" in notification and "text" in notification["params"]:
                text = notification["params"]["text"]
                print(f"Transcription text: '{text}'")
            break

    if not notification_received:
        print("No segment.final notification received (this is normal if recording was silent)")

    sock.close()

    print("\n=== Summary ===")
    if success:
        print("✓ Both session.start and session.stop responses received with 'result'")
        sys.exit(0)
    else:
        print("✗ Error: One or more responses contained 'error' or was not received")
        sys.exit(1)

if __name__ == "__main__":
    main()
