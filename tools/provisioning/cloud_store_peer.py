import argparse
import contextlib
import sys
from pathlib import Path

from receipt_store import ReceiptStore, StoreError


MAX_LINE = 4096


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--peer", nargs=3, action="append", required=True)
    parser.add_argument("--checkpoint")
    args = parser.parse_args()
    policies = {}
    for principal, anchor, resource in args.peer:
        principal, anchor, resource = map(bytes.fromhex, (principal, anchor, resource))
        if len(principal) != 32 or len(anchor) != 72 or len(resource) != 32:
            raise ValueError("invalid complete fixture policy")
        if principal in policies:
            raise ValueError("duplicate fixture principal")
        policies[principal] = (anchor, resource)
    if len(policies) != 2:
        raise ValueError("fixture requires two distinct authenticated principals")

    @contextlib.contextmanager
    def authorize(principal, anchor, resource, action):
        if (policies.get(principal) != (anchor, resource)
                or action not in {"telemetry", "desired", "applied", "management"}):
            raise StoreError("unauthorized fixture principal or policy")
        yield

    args.database.parent.mkdir(parents=True, exist_ok=True)
    store = ReceiptStore(
        args.database,
        authorize,
        bytes.fromhex(args.checkpoint) if args.checkpoint else None,
    )
    for index, (principal, (anchor, resource)) in enumerate(policies.items(), 1):
        store.set_desired(principal, anchor, resource, 1, b"device-config-" + bytes([index, 0, 255]))
    print("ready", flush=True)
    try:
        while True:
            line = sys.stdin.buffer.readline(MAX_LINE + 1)
            if not line:
                break
            if len(line) > MAX_LINE or not line.endswith(b"\n"):
                raise ValueError("provider line exceeds fixed profile")
            parts = line.decode("ascii").strip().split()
            if parts == ["stop"]:
                print("ok", flush=True)
                break
            try:
                if parts == ["checkpoint"]:
                    result = store.checkpoint()
                elif parts == ["count"]:
                    result = store.db.execute("SELECT COUNT(*) FROM receipts").fetchone()[0].to_bytes(8, "big")
                elif parts[0] == "telemetry" and len(parts) == 8:
                    principal, anchor, resource, operation, digest = map(bytes.fromhex, parts[1:6])
                    content_format = int(parts[6])
                    payload = b"" if parts[7] == "-" else bytes.fromhex(parts[7])
                    result = store.commit(principal, anchor, operation, resource, content_format, digest, payload)
                elif parts[0] == "desired" and len(parts) == 4:
                    principal, anchor, resource = map(bytes.fromhex, parts[1:4])
                    desired = store.get_desired(principal, anchor, resource)
                    if desired is None:
                        raise StoreError("missing desired state")
                    result = desired[0].to_bytes(8, "big") + desired[1]
                elif parts[0] == "applied" and len(parts) == 5:
                    principal, anchor, resource = map(bytes.fromhex, parts[1:4])
                    version = int(parts[4])
                    store.report_applied(principal, anchor, resource, version)
                    result = version.to_bytes(8, "big")
                else:
                    raise ValueError("invalid complete provider operation")
                print("ok " + result.hex(), flush=True)
            except StoreError as error:
                message = str(error)
                if "conflict" in message:
                    print("conflict", flush=True)
                elif "unauthorized" in message:
                    print("unauthorized", flush=True)
                elif "capacity" in message:
                    print("capacity", flush=True)
                else:
                    print("error", flush=True)
            except (ValueError, IndexError, OverflowError):
                print("error", flush=True)
    finally:
        store.close()


if __name__ == "__main__":
    main()
