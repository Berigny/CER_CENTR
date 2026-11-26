"""evidence/0 for Python.

Portable evidence objects for records kept in other systems. This package is a
thin wrapper over the Rust core, so every result is byte-for-byte the same as
the ``evidence`` command-line tool and the WebAssembly module.

Pass file contents as ``bytes`` (exact) or ``str`` (encoded as UTF-8)::

    import evidence_layer as ev

    report = ev.verify(open("object.json", "rb").read(),
                       attestations=open("attestations.jsonl", "rb").read(),
                       require=["approve"])
    if not report["ok"]:
        print(report["code"], report["detail"])
"""

from __future__ import annotations

import json
import os
from datetime import datetime, timezone
from typing import Any, Dict, Iterable, Optional, Union

from . import _native

__all__ = [
    "SPEC_VERSION",
    "PURPOSES",
    "EvidenceError",
    "canonicalize",
    "content_hash",
    "file_hash",
    "compute_id",
    "seal",
    "verify",
    "sign",
    "public_key",
    "generate_seed",
    "now",
]

SPEC_VERSION: str = _native.SPEC_VERSION
__version__: str = _native.CRATE_VERSION
PURPOSES = ("author", "approve", "witness")

Data = Union[str, bytes, bytearray, memoryview]


class EvidenceError(ValueError):
    """A verification or usage error with a stable code, such as ``E_ID_MISMATCH``.

    ``E_USAGE`` means the arguments were invalid (a malformed seed, an unknown
    purpose, an invalid store object); every other code is from the spec.
    """

    def __init__(self, code: str, detail: str) -> None:
        super().__init__(f"{code}: {detail}")
        self.code = code
        self.detail = detail


def _bytes(data: Data) -> bytes:
    if isinstance(data, str):
        return data.encode("utf-8")
    return bytes(data)


def _call(fn: Any, *args: Any) -> Any:
    try:
        return fn(*args)
    except _native.EvidenceError as e:
        code, detail = e.args
        raise EvidenceError(code, detail) from None


def canonicalize(data: Data) -> str:
    """Canonical JSON (RFC 8785) of any JSON text, under the strict parsing rules."""
    return _call(_native.canonicalize, _bytes(data))


def content_hash(data: Data) -> str:
    """``content.hash`` for a JSON record: sha256 of its canonical form."""
    return _call(_native.content_hash, _bytes(data))


def file_hash(data: Data) -> str:
    """sha256 of raw bytes: ``schema.hash``, and ``content.hash`` for non-JSON content."""
    return _call(_native.file_hash, _bytes(data))


def compute_id(data: Data) -> str:
    """The id an object should have."""
    return _call(_native.compute_id, _bytes(data))


def seal(data: Data) -> str:
    """The object with its id filled in, as canonical JSON. Call :func:`verify` to check it."""
    return _call(_native.seal, _bytes(data))


def verify(
    data: Data,
    *,
    attestations: Optional[Data] = None,
    store: Optional[Iterable[Data]] = None,
    require: Iterable[str] = (),
) -> Dict[str, Any]:
    """Verify an object and return the report.

    The report has ``ok``, ``id``, ``code`` and ``detail`` (on failure),
    ``content_checked`` and ``attestations`` (valid ones, by purpose).
    Verification failures are in the report; :class:`EvidenceError` is raised
    only for invalid arguments.

    ``store`` holds the objects that ``prev`` may point at. Leave it as
    ``None`` to skip the prev check; an empty list means an empty store.
    """
    report = _call(
        _native.verify,
        _bytes(data),
        None if attestations is None else _bytes(attestations),
        None if store is None else [_bytes(s) for s in store],
        list(require),
    )
    return json.loads(report)


def sign(
    data: Data,
    *,
    seed_hex: str,
    signer: str,
    purpose: str,
    at: Optional[str] = None,
) -> str:
    """Sign a valid object. Returns one attestation line, without the newline.

    ``at`` defaults to :func:`now`. If the object is invalid, its verification
    error is raised.
    """
    return _call(_native.sign, _bytes(data), seed_hex, signer, purpose, at or now())


def public_key(seed_hex: str) -> str:
    """The ``ed25519:`` public key for a hex seed."""
    return _call(_native.public_key, seed_hex)


def generate_seed() -> str:
    """A new random 32-byte seed as hex. For development; keep production keys in an HSM."""
    return os.urandom(32).hex()


def now() -> str:
    """The current UTC time as ``YYYY-MM-DDTHH:MM:SSZ``."""
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")
