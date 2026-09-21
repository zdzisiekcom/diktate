#!/usr/bin/env python3
"""
Smoke test for diktate daemon IPC communication.
Sends session.start, waits 2 seconds, sends session.stop,
and waits for optional segment.added notification.
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

def send_message(sock, method, msg_id, params=None):
    """Send a JSON-RPC message."""
    message = {
        "jsonrpc": "2.0",
        "id": msg_id,
        "method": method
    }
    if params is not None:
        message["params"] = params
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

def read_response(sock, timeout=None):
    """Czyta odpowiedź z gniazda, pomijając powiadomienia po drodze.

    Daemon miesza na jednym gnieździe odpowiedzi (mają pole 'id') z
    powiadomieniami — state.changed, segment.added, audio.level — które
    mają 'method', a nie 'id', i mogą przyjść w dowolnej chwili między
    wysłaniem żądania a jego odpowiedzią (docs/02). Ta funkcja czyta kolejne
    linie, wypisuje i odrzuca każdą bez 'id', i zwraca dopiero tę pierwszą,
    która wygląda na odpowiedź.
    """
    deadline = None if timeout is None else time.time() + timeout
    while True:
        if deadline is not None:
            remaining = deadline - time.time()
            if remaining <= 0:
                return None
        else:
            remaining = None
        message = read_message(sock, timeout=remaining)
        if message is None:
            return None
        if "id" not in message:
            print(f"  (pominięto powiadomienie: {json.dumps(message)})")
            continue
        return message

def main():
    socket_path = get_socket_path()
    print(f"Connecting to {socket_path}...")

    sock = connect_to_daemon(socket_path)
    print("Connected.\n")

    success = True

    # Wyślij session.context z tekstem — daemon ma zwrócić liczbę znaków.
    print("=== Faza 0a: session.context (z tekstem) ===")
    kontekst = "Terminy: ggml, Wayland.\nOstatnie zdanie notatki."
    send_message(sock, "session.context", 1, {"text": kontekst})
    response_context = read_response(sock, timeout=5)

    if response_context is None:
        print("← Błąd: brak odpowiedzi na session.context")
        success = False
    else:
        print(f"← Otrzymano: {json.dumps(response_context)}")
        chars = response_context.get("result", {}).get("chars")
        if chars != 48:
            print(f"Błąd: oczekiwano chars == 48, otrzymano {chars!r}")
            success = False

    # Wyślij session.context z pustym tekstem — slot ma się wyczyścić.
    print("\n=== Faza 0b: session.context (pusty tekst) ===")
    send_message(sock, "session.context", 2, {"text": ""})
    response_context_empty = read_response(sock, timeout=5)

    if response_context_empty is None:
        print("← Błąd: brak odpowiedzi na session.context (pusty tekst)")
        success = False
    else:
        print(f"← Otrzymano: {json.dumps(response_context_empty)}")
        chars = response_context_empty.get("result", {}).get("chars")
        if chars != 0:
            print(f"Błąd: oczekiwano chars == 0, otrzymano {chars!r}")
            success = False

    # Send session.start
    print("\n=== Phase 1: session.start ===")
    send_message(sock, "session.start", 3)
    response1 = read_response(sock, timeout=5)

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
    send_message(sock, "session.stop", 4)
    response2 = read_response(sock, timeout=5)

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

    # Wait for optional segment.added notification
    print("\n=== Phase 3: Waiting for segment.added notification (up to 60 seconds) ===")
    start_time = time.time()
    notification_received = False

    while time.time() - start_time < 60:
        remaining = 60 - (time.time() - start_time)
        notification = read_message(sock, timeout=remaining)

        if notification is None:
            break

        print(f"← Received: {json.dumps(notification)}")

        if "method" in notification and notification["method"] == "segment.added":
            notification_received = True
            if "params" in notification and "text" in notification["params"]:
                text = notification["params"]["text"]
                print(f"Transcription text: '{text}'")
            break

    if not notification_received:
        print("No segment.added notification received (this is normal if recording was silent)")

    sock.close()

    print("\n=== Summary ===")
    if success:
        print("✓ session.context, session.start i session.stop odpowiedziały poprawnie")
        sys.exit(0)
    else:
        print("✗ Error: One or more responses contained 'error' or was not received")
        sys.exit(1)

if __name__ == "__main__":
    main()
