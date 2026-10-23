#!/usr/bin/env python3
"""Writes the synthetic agy fixtures under tests/fixtures/sessions/agy/
(plan 20261023_06, step 1, as amended): databases with agy's `steps`
table and the step types and protobuf fields of docs/AGY_STEPS.md, holding
made-up text. Nothing here comes from a real conversation.

    scripts/make-agy-fixtures.py
"""
import os
import sqlite3

CONVERSATION = "00000000-0000-4000-8000-00000000a6e1"


def varint(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def field(number, value):
    """One field: an int as a varint, bytes or str as length-delimited."""
    if isinstance(value, int):
        return varint(number << 3) + varint(value)
    if isinstance(value, str):
        value = value.encode()
    return varint(number << 3 | 2) + varint(len(value)) + value


def msg(*fields):
    return b"".join(fields)


def header(idx, seconds, call=None):
    """Field 5: when (1), the call a result answers (4), where (20)."""
    parts = [
        field(1, msg(field(1, seconds), field(2, 0))),
        field(20, msg(field(2, idx), field(4, CONVERSATION))),
    ]
    if call:
        parts.insert(1, field(4, msg(field(1, call[0]), field(2, call[1]))))
    return field(5, msg(*parts))


def user(idx, t, text):
    return 14, msg(field(1, 14), field(4, 3), header(idx, t), field(19, msg(field(2, text))))


def response(idx, t, reply=None, narration=None, calls=()):
    body = [field(6, "bot-00000000-0000-4000-8000-000000000001")]
    if reply is not None:
        body += [field(1, reply), field(8, reply)]
    if narration is not None:
        body.append(field(3, narration))
    for call_id, name, args in calls:
        body.append(field(7, msg(field(1, call_id), field(2, name), field(3, args))))
    body.append(field(12, 2))
    return 15, msg(field(1, 15), field(4, 3), header(idx, t), field(20, msg(*body)))


def result(idx, t, call_id, name, output):
    out = field(140, msg(field(2, msg(field(1, output)))))
    return 132, msg(field(1, 132), field(4, 3), header(idx, t, (call_id, name)), out)


def snapshot(idx, t):
    """A state snapshot (type 23): memory-like, which the reader leaves out."""
    return 23, msg(field(1, 23), field(4, 3), header(idx, t), field(30, b"\x0a\x04none"))


def write(path, steps):
    if os.path.exists(path):
        os.remove(path)
    db = sqlite3.connect(path)
    db.execute(
        "CREATE TABLE `steps` (`idx` integer,`step_type` integer NOT NULL DEFAULT 0,"
        "`status` integer NOT NULL DEFAULT 0,`has_subtrajectory` numeric NOT NULL DEFAULT false,"
        "`metadata` blob,`error_details` blob,`permissions` blob,`task_details` blob,"
        "`render_info` blob,`step_payload` blob,`step_format` integer NOT NULL DEFAULT 0,"
        "PRIMARY KEY (`idx`))"
    )
    for idx, (step_type, payload) in enumerate(steps):
        db.execute(
            "INSERT INTO steps (idx, step_type, status, step_payload) VALUES (?, ?, 3, ?)",
            (idx, step_type, payload),
        )
    db.commit()
    db.execute("VACUUM")
    db.close()


def main():
    here = os.path.join(os.path.dirname(__file__), "..", "tests", "fixtures", "sessions", "agy")
    os.makedirs(here, exist_ok=True)
    t = 1_790_000_000
    common = [
        user(0, t, "is the docs site up?"),
        response(1, t + 2, narration="Checking the server.", calls=[
            ("call_1", "run_command", '{"CommandLine":"make status"}')]),
        result(2, t + 4, "call_1", "run_command", "server running on port 4000"),
        snapshot(3, t + 5),
    ]
    write(os.path.join(here, "asking.db"), common + [
        response(4, t + 6, reply="The docs site is up on port 4000.\n\nShall I open it in the browser?"),
    ])
    write(os.path.join(here, "stopped.db"), common + [
        response(4, t + 6, reply="The docs site is up on port 4000."),
    ])


if __name__ == "__main__":
    main()
