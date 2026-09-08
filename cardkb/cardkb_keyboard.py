"""M5Stack CardKB -> Virtual USB keyboard via I2C + evdev/uinput.

CardKB sends fully-formed ASCII over I2C at fixed address 0x5F.
This script polls the CardKB and re-emits each byte as a Linux
input event, so the whole OS sees it as a real keyboard.

Run:  python3 cardkb_keyboard.py
Test: focus a text editor/terminal and press CardKB keys.
Verify: cat /proc/bus/input/devices | grep -A5 CardKB
"""

import smbus2
import time
from evdev import UInput, ecodes as e

BUS_ID = 1
ADDR = 0x5F
POLL_DELAY = 0.03

# Special non-ASCII key codes from the CardKB manual
SPECIAL_KEYS = {
    0x08: (e.KEY_BACKSPACE, False),
    0x09: (e.KEY_TAB, False),
    0x0D: (e.KEY_ENTER, False),
    0x1B: (e.KEY_ESC, False),
    0xB4: (e.KEY_LEFT, False),
    0xB5: (e.KEY_UP, False),
    0xB6: (e.KEY_DOWN, False),
    0xB7: (e.KEY_RIGHT, False),
    0xB8: (e.KEY_DOWN, False),   # alias seen on some fw revisions
    0xB9: (e.KEY_RIGHT, False),  # alias seen on some fw revisions
}

# Printable ASCII -> (keycode, needs_shift)
CHAR_MAP = {
    'a': (e.KEY_A, False), 'b': (e.KEY_B, False), 'c': (e.KEY_C, False),
    'd': (e.KEY_D, False), 'e': (e.KEY_E, False), 'f': (e.KEY_F, False),
    'g': (e.KEY_G, False), 'h': (e.KEY_H, False), 'i': (e.KEY_I, False),
    'j': (e.KEY_J, False), 'k': (e.KEY_K, False), 'l': (e.KEY_L, False),
    'm': (e.KEY_M, False), 'n': (e.KEY_N, False), 'o': (e.KEY_O, False),
    'p': (e.KEY_P, False), 'q': (e.KEY_Q, False), 'r': (e.KEY_R, False),
    's': (e.KEY_S, False), 't': (e.KEY_T, False), 'u': (e.KEY_U, False),
    'v': (e.KEY_V, False), 'w': (e.KEY_W, False), 'x': (e.KEY_X, False),
    'y': (e.KEY_Y, False), 'z': (e.KEY_Z, False),
    ' ': (e.KEY_SPACE, False),
    '0': (e.KEY_0, False), '1': (e.KEY_1, False), '2': (e.KEY_2, False),
    '3': (e.KEY_3, False), '4': (e.KEY_4, False), '5': (e.KEY_5, False),
    '6': (e.KEY_6, False), '7': (e.KEY_7, False), '8': (e.KEY_8, False),
    '9': (e.KEY_9, False),
    ',': (e.KEY_COMMA, False), '.': (e.KEY_DOT, False),
    '!': (e.KEY_1, True), '@': (e.KEY_2, True), '#': (e.KEY_3, True),
    '$': (e.KEY_4, True), '%': (e.KEY_5, True), '^': (e.KEY_6, True),
    '&': (e.KEY_7, True), '*': (e.KEY_8, True), '(': (e.KEY_9, True),
    ')': (e.KEY_0, True),
    '{': (e.KEY_LEFTBRACE, True), '}': (e.KEY_RIGHTBRACE, True),
    '[': (e.KEY_LEFTBRACE, False), ']': (e.KEY_RIGHTBRACE, False),
    '/': (e.KEY_SLASH, False), '\\': (e.KEY_BACKSLASH, False),
    '|': (e.KEY_BACKSLASH, True), '~': (e.KEY_GRAVE, True),
    "'": (e.KEY_APOSTROPHE, False), '"': (e.KEY_APOSTROPHE, True),
    ';': (e.KEY_SEMICOLON, False), ':': (e.KEY_SEMICOLON, True),
    '`': (e.KEY_GRAVE, False), '+': (e.KEY_EQUAL, True),
    '-': (e.KEY_MINUS, False), '_': (e.KEY_MINUS, True),
    '=': (e.KEY_EQUAL, False), '?': (e.KEY_SLASH, True),
    '<': (e.KEY_COMMA, True), '>': (e.KEY_DOT, True),
}
# Uppercase = same key + shift
for ch, (kc, shift) in list(CHAR_MAP.items()):
    if ch.isalpha():
        CHAR_MAP[ch.upper()] = (kc, True)

ALL_KEYCODES = list(
    {kc for kc, _ in CHAR_MAP.values()}
    | {kc for kc, _ in SPECIAL_KEYS.values()}
    | {e.KEY_LEFTSHIFT}
)


def send_key(ui: UInput, keycode: int, shift: bool = False) -> None:
    if shift:
        ui.write(e.EV_KEY, e.KEY_LEFTSHIFT, 1)
    ui.write(e.EV_KEY, keycode, 1)
    ui.syn()
    ui.write(e.EV_KEY, keycode, 0)
    if shift:
        ui.write(e.EV_KEY, e.KEY_LEFTSHIFT, 0)
    ui.syn()


def main() -> None:
    bus = smbus2.SMBus(BUS_ID)
    ui = UInput({e.EV_KEY: ALL_KEYCODES}, name="CardKB-Virtual-Keyboard")
    print("CardKB virtual keyboard running. Press Ctrl+C to quit.")
    try:
        while True:
            try:
                data = bus.read_byte(ADDR)
            except OSError:
                # NACK / bus hiccup: just poll again
                time.sleep(POLL_DELAY)
                continue
            if data != 0:
                if data in SPECIAL_KEYS:
                    send_key(ui, *SPECIAL_KEYS[data])
                else:
                    try:
                        ch = chr(data)
                    except ValueError:
                        ch = ""
                    if ch in CHAR_MAP:
                        send_key(ui, *CHAR_MAP[ch])
                    else:
                        print(f"unmapped CardKB byte: 0x{data:02X}")
            time.sleep(POLL_DELAY)
    except KeyboardInterrupt:
        print("\nExiting.")
    finally:
        try:
            ui.close()
        except Exception:
            pass


if __name__ == "__main__":
    main()
