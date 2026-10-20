"""The seeding policy (Stage G §5, locked).

Every generated item gets a seed derived from the master seed and the
item's identity, so any one circuit can be regenerated alone, in any order,
on any machine::

    seed = first 8 bytes (big-endian) of SHA-256("master|family|n|reps|instance")

No other randomness is used anywhere in the harness.
"""

from __future__ import annotations

import hashlib


def item_seed(master: int, family: str, n: int, reps: int = 0, instance: int = 0) -> int:
    """The 64-bit seed of one benchmark item."""
    text = f"{master}|{family}|{n}|{reps}|{instance}".encode()
    return int.from_bytes(hashlib.sha256(text).digest()[:8], "big")
