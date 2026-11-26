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

The store/0 functions check log entries, Merkle proofs and whole exports::

    store_report = ev.verify_export(open("export/manifest.json", "rb").read(),
                                    [open("export/000001.jsonl", "rb").read()])
    for finding in store_report["findings"]:
        print(finding["code"], finding.get("seq"))
"""

from __future__ import annotations

import json
import os
from datetime import datetime, timezone
from typing import Any, Dict, Iterable, List, Optional, Union

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
    "entry_hash",
    "merkle_root",
    "verify_inclusion",
    "verify_consistency",
    "verify_export",
]

SPEC_VERSION: str = _native.SPEC_VERSION
__version__: str = _native.CRATE_VERSION
PURPOSES = ("author", "approve", "witness")

Data = Union[str, bytes, bytearray, memoryview]

# The largest index, size or max_findings the store functions take.
_MAX_INTEGER = 2**53 - 1


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


# store/0 (spec/store-0.md, "Programming interfaces"). Arguments are checked
# here, so that a wrong type is EvidenceError E_USAGE, not TypeError.


def _usage(detail: str) -> EvidenceError:
    return EvidenceError("E_USAGE", detail)


def _data(value: Any, name: str) -> bytes:
    if isinstance(value, str):
        try:
            return value.encode("utf-8")
        except UnicodeEncodeError:
            raise _usage(f"{name} is not valid Unicode text") from None
    if isinstance(value, (bytes, bytearray, memoryview)):
        return bytes(value)
    raise _usage(f"{name} must be bytes or str")


def _data_list(values: Any, name: str) -> List[bytes]:
    if isinstance(values, (str, bytes, bytearray, memoryview)):
        raise _usage(f"{name} must be a list of bytes or str")
    try:
        items = list(values)
    except TypeError:
        raise _usage(f"{name} must be a list of bytes or str") from None
    return [_data(v, f"{name} items") for v in items]


def _integer(value: Any, name: str, low: int) -> int:
    if isinstance(value, bool) or not isinstance(value, int):
        raise _usage(f"{name} must be an integer from {low} to 2**53 - 1")
    if not low <= value <= _MAX_INTEGER:
        raise _usage(f"{name} must be an integer from {low} to 2**53 - 1")
    return value


def _hash(value: Any, name: str) -> str:
    if not isinstance(value, str):
        raise _usage(f'{name} must be "sha256:" and 64 lowercase hex digits')
    return value


def _hash_list(values: Any, name: str) -> List[str]:
    if isinstance(values, (str, bytes, bytearray, memoryview)):
        raise _usage(f"{name} must be a list of hashes")
    try:
        items = list(values)
    except TypeError:
        raise _usage(f"{name} must be a list of hashes") from None
    return [_hash(v, f"{name} items") for v in items]


def entry_hash(data: Data) -> str:
    """The entry hash of a log-entry/0 entry: sha256 of its canonical form.

    Raises :class:`EvidenceError` ``E_USAGE``, with a detail that begins with
    ``S_ENTRY_INVALID``, if ``data`` is not a valid entry.
    """
    return _call(_native.entry_hash, _data(data, "data"))


def merkle_root(leaves: Iterable[Data]) -> str:
    """The RFC 9162 tree hash of these leaf bytes. Each is hashed as a leaf first.

    An empty list gives the hash of the empty tree, sha256 of no bytes.
    """
    return _call(_native.merkle_root, _data_list(leaves, "leaves"))


def verify_inclusion(
    leaf_hash: str,
    index: int,
    size: int,
    proof: Iterable[str],
    root: str,
) -> bool:
    """Check an inclusion proof: that the leaf with this leaf hash is at
    0-based ``index`` (``seq - 1``) in the tree of ``size`` leaves with this root.

    ``leaf_hash`` is sha256 of ``0x00`` and the canonical entry, as ``store
    prove`` prints it. Hashes are text, ``"sha256:"`` and 64 lowercase hex digits.
    """
    return _call(
        _native.verify_inclusion,
        _hash(leaf_hash, "leaf_hash"),
        _integer(index, "index", 0),
        _integer(size, "size", 0),
        _hash_list(proof, "proof"),
        _hash(root, "root"),
    )


def verify_consistency(
    old_size: int,
    new_size: int,
    old_root: str,
    new_root: str,
    proof: Iterable[str],
) -> bool:
    """Check a consistency proof: that the tree of ``old_size`` leaves with root
    ``old_root`` is a prefix of the tree of ``new_size`` leaves with root ``new_root``.
    """
    return _call(
        _native.verify_consistency,
        _integer(old_size, "old_size", 0),
        _integer(new_size, "new_size", 0),
        _hash(old_root, "old_root"),
        _hash(new_root, "new_root"),
        _hash_list(proof, "proof"),
    )


def verify_export(
    manifest: Data,
    segments: Iterable[Data],
    max_findings: int = 1000,
) -> Dict[str, Any]:
    """Verify an export and return its ``store-report/0`` report.

    ``manifest`` is the bytes of ``manifest.json`` and ``segments`` the bytes
    of the segment files, in manifest order. The report keeps at most
    ``max_findings`` findings. Findings are in the report, however damaged
    the export is; :class:`EvidenceError` is raised only for invalid arguments.
    """
    report = _call(
        _native.verify_export,
        _data(manifest, "manifest"),
        _data_list(segments, "segments"),
        _integer(max_findings, "max_findings", 1),
    )
    return json.loads(report)
