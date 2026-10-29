#!/usr/bin/env python3
import hashlib
import json
import math
import os
from pathlib import Path
import re
import selectors
import shutil
import signal
import subprocess
import sys
import time
import tomllib
BASE = "869d4354f4a29158fe3644b8d5d71b9b0bb591d6"
TEST = "oscore::protect::binding_cost_probe::criterion_probe"
METRICS = ("normal-entry", "first-slot-entry", "fallback-entry", "normal-lookup")
ARMS = ((1, "baseline"), (1, "candidate"), (2, "candidate"), (2, "baseline"))
# Cargo selects LTO per crate type; global -C lto rejects proc-macro dependencies.
FLAGS = ""
MIB = 1024 * 1024
ART_CAP = 2 * MIB
RAW_CAP = 256 * 1024
SOURCE = ("src/oscore/context.rs", "src/oscore/protect.rs", "src/oscore/tests.rs")
ASSETS = ("tools/experiments/binding-only/hosted_probe.py",
     "tools/experiments/binding-only/criterion_probe.rs",
     "tools/experiments/binding-only/measurement-plan.json", ".github/workflows/ci.yml")
ROOT = Path(os.environ["RUNNER_TEMP"]).resolve() / "binding-probe"
EVIDENCE = ROOT / "evidence"
REPO = Path.cwd().resolve()
ENV = os.environ.copy()
STATE = {"status": "INCONCLUSIVE", "correctness": "NOTRUN", "arms": [
  {"round": n, "arm": arm, "status": "NOTRUN"} for n, arm in ARMS]}
OWNED = False
META = 0
def require(ok, detail):
  if not ok:
    raise RuntimeError(detail)
def inventory(root, cap, count):
  total = 0
  files = []
  for folder, dirs, names in os.walk(root, followlinks=False):
    for name in dirs + names:
      require(not (Path(folder) / name).is_symlink(), "symlink in owned output")
    for name in names:
      p = Path(folder) / name
      require(p.is_file(), "non-file in owned output")
      total += p.stat().st_size
      files.append(p)
      require(total <= cap and len(files) <= count, "owned output budget exceeded")
  return total, files
def save(name, data):
  if not isinstance(data, bytes):
    data = (json.dumps(data, sort_keys=True, indent=2, allow_nan=False) + "\n").encode()
  p = EVIDENCE / name
  require(p.is_relative_to(EVIDENCE), "evidence path escapes root")
  used, _ = inventory(EVIDENCE, ART_CAP, 512)
  previous = p.stat().st_size if p.exists() else 0
  require(used - previous + len(data) <= ART_CAP - 8192, "evidence budget reserve reached")
  p.parent.mkdir(parents=True, exist_ok=True)
  p.write_bytes(data)
def digest(p):
  h = hashlib.sha256()
  with Path(p).open("rb") as f:
    while block := f.read(MIB):
      h.update(block)
  return h.hexdigest()
def resource_guard():
  a, _ = inventory(ROOT / "target", 8 * 1024**3, 100000)
  b, _ = inventory(ROOT / "cargo-home", 8 * 1024**3, 100000)
  require(a + b <= 8 * 1024**3, "target plus Cargo home exceeds 8GiB")
  require(shutil.disk_usage(ROOT).free >= 4 * 1024**3, "free space below 4GiB")
def group_exists(pgid):
  try:
    os.killpg(pgid, 0)
    return True
  except ProcessLookupError:
    return False
def observed_exit(proc):
  # WNOWAIT reserves the leader's PID/PGID until our final group signal.
  return os.waitid(os.P_PID, proc.pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)

def run(args, name, timeout, cap=65536, env=None, building=False, cpu=None):
  used, _ = inventory(EVIDENCE, ART_CAP, 512)
  require(used + cap <= ART_CAP - 8192, "insufficient reserved log capacity")
  outputs = [EVIDENCE / (name + ".stdout.log"), EVIDENCE / (name + ".stderr.log")]
  files = [p.open("wb") for p in outputs]
  proc = None
  sel = selectors.DefaultSelector()
  total = 0
  deadline = time.monotonic() + timeout
  next_guard = 0.0
  def pin():
    if cpu is not None:
      os.sched_setaffinity(0, {cpu})
      require(os.sched_getaffinity(0) == {cpu}, "affinity pin failed")
  try:
    proc = subprocess.Popen(args, cwd=REPO, env=env or ENV,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                start_new_session=True, preexec_fn=pin)
    for i, stream in enumerate((proc.stdout, proc.stderr)):
      os.set_blocking(stream.fileno(), False)
      sel.register(stream, selectors.EVENT_READ, i)
    while sel.get_map() or observed_exit(proc) is None:
      now = time.monotonic()
      require(now < deadline, name + " timeout")
      if building and now >= next_guard:
        resource_guard()
        next_guard = now + 5
      for key, _ in sel.select(min(0.2, max(0, deadline - now))):
        block = os.read(key.fileobj.fileno(), 8192)
        if not block:
          sel.unregister(key.fileobj)
          continue
        require(total + len(block) <= cap, name + " combined log overflow")
        total += len(block)
        files[key.data].write(block)
        files[key.data].flush()
        dest = sys.stdout.buffer if key.data == 0 else sys.stderr.buffer
        dest.write(block)
        dest.flush()
    status = observed_exit(proc)
    require(status is not None, name + " missing exit status")
    code = status.si_status if status.si_code == os.CLD_EXITED else -status.si_status
    require(code == 0, name + " exit " + str(code))
  finally:
    try:
      if proc is not None:
        # No poll/wait has reaped this leader. Never signal this numeric PGID
        # after wait(), even if the later read-only settlement check fails.
        try:
          os.killpg(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
          pass
        proc.wait(timeout=5)
        settled = time.monotonic() + 5
        while group_exists(proc.pid) and time.monotonic() < settled:
          time.sleep(0.01)
        require(not group_exists(proc.pid), name + " owned process group cleanup unresolved")
    finally:
      sel.close()
      for f in files:
        f.close()
  return outputs[0].read_bytes()
def small(args):
  global META
  META += 1
  return run(args, "meta-" + str(META), 15).decode().strip()
def method(text, name):
  matches = list(re.finditer(r"(?m)^    pub(?:\(crate\))? fn " + name + r"(?:<[^\n]*>)?\(", text))
  require(len(matches) == 1, "missing/ambiguous baseline method " + name)
  start = matches[0].start()
  opening = text.index("{", matches[0].end())
  depth = 1
  pos = opening + 1
  while depth:
    require(pos < len(text), "unterminated method")
    # These pinned methods contain no body literals or block comments. Skip
    # line comments without interpreting their braces, preserving original bytes.
    if text.startswith("//", pos):
      pos = text.find("\n", pos)
      require(pos >= 0, "unterminated method comment")
      continue
    require(text[pos] not in "\"'/", "unsupported literal/block comment in method")
    depth += (text[pos] == "{") - (text[pos] == "}")
    pos += 1
  return text[start:pos], opening - start
def lock_guard(original, selected):
  old = tomllib.loads(original.decode())["package"]
  new = tomllib.loads(selected.decode())["package"]
  key = lambda p: (p["name"], p["version"], p.get("source"))
  indexed = {key(p): p for p in new}
  require(len(indexed) == len(new), "duplicate selected lock packages")
  for package in old:
    updated = indexed.get(key(package))
    require(updated is not None, "original package missing from selected lock")
    if package["name"] == "coaptic" and "source" not in package:
      expected = dict(package)
      expected["dependencies"] = sorted(package.get("dependencies", []) + ["criterion"])
      require(updated == expected, "root lock change beyond Criterion edge")
    else:
      require(updated == package, "original lock entry/checksum/edges changed: " + package["name"])
  old_names = {p["name"] for p in old}
  for package in new:
    require(package["name"] not in old_names or key(package) in {key(p) for p in old},
        "new version of existing dependency, including crypto/syn, refused")
  require(any(p["name"] == "criterion" and p["version"] == "0.5.1" for p in new),
      "Criterion version not selected")
def finite(value):
  require(isinstance(value, (float, int)) and not isinstance(value, bool)
      and math.isfinite(value), "nonfinite estimate")
  return float(value)
def interval(estimate):
  ci = estimate["confidence_interval"]
  require(ci["confidence_level"] == 0.95, "wrong confidence level")
  lo, point, hi = (finite(ci["lower_bound"]), finite(estimate["point_estimate"]),
          finite(ci["upper_bound"]))
  require(lo <= point <= hi, "invalid interval")
  return {"point": point, "lower": lo, "upper": hi}
def normalize(estimate, inverse):
  d = interval(estimate)
  require(d["lower"] > -1, "nonpositive ratio")
  if inverse:
    return {"point": 1 / (1 + d["point"]) - 1,
        "lower": 1 / (1 + d["upper"]) - 1,
        "upper": 1 / (1 + d["lower"]) - 1}
  return d
def explicit_empty_fixture_slices(text):
  # Criterion brings serde_json's additional PartialEq implementations into
  # libtest. Preserve these existing assertions while fixing their byte type.
  old = (".get_option(OptionNumber::NO_RESPONSE)\n"
         "                        .unwrap()\n"
         "                        .value(),\n"
         "                    &[]")
  require(text.count(old) == 2, "unexpected empty-slice fixture sites")
  new = old + " as &[u8]"
  return text.replace(old, new), old, new

def decide():
  result = {}
  for n in (1, 2):
    row = {}
    for metric in METRICS:
      directory = ROOT / "criterion" / ("round" + str(n)) / metric
      read = lambda path: json.loads((directory / path).read_bytes())
      change = normalize(read("change/estimates.json")["mean"], n == 2)
      estimates = {}
      for saved in ("first", "new"):
        sample = read(saved + "/sample.json")
        require(len(sample["iters"]) == len(sample["times"]) == 30,
            "not exactly 30 Criterion samples")
        mean = interval(read(saved + "/estimates.json")["mean"])
        require(mean["lower"] > 0, "nonpositive measured mean")
        estimates[saved] = mean
        read(saved + "/benchmark.json")
      row[metric] = {"candidate_vs_baseline": change, "raw_mean_ns": estimates,
             "first_arm": "baseline" if n == 1 else "candidate"}
    result[str(n)] = row
  accept = all(
    row["normal-entry"]["candidate_vs_baseline"]["point"] <= -0.02
    and row["normal-entry"]["candidate_vs_baseline"]["upper"] < 0
    and row["normal-lookup"]["candidate_vs_baseline"]["point"] < 0
    and row["fallback-entry"]["candidate_vs_baseline"]["upper"] <= 0.02
    and row["first-slot-entry"]["candidate_vs_baseline"]["upper"] <= 0.02
    for row in result.values())
  STATE.update(status="ACCEPT" if accept else "INCONCLUSIVE/REJECT", rounds=result,
        scope="OSCORE binding probe only; no socket, CPU50 or whole-memory claim")
def main():
  global OWNED
  require(sys.platform == "linux", "hosted Linux only")
  require(not ROOT.exists(), "owned root already exists; no retries")
  EVIDENCE.mkdir(parents=True)
  OWNED = True
  require(not small(["git", "status", "--porcelain", "--untracked-files=all"]),
      "checkout is not clean before injection")
  commit = small(["git", "rev-parse", "HEAD"])
  tree = small(["git", "rev-parse", "HEAD^{tree}"])
  require(re.fullmatch(r"[0-9a-f]{40}", commit) is not None, "invalid commit")
  changed = small(["git", "diff", "--name-only", BASE, "HEAD"]).splitlines()
  require(set(changed) <= set(SOURCE + ASSETS), "production/temporary diff outside whitelist")
  tracked = small(["git", "ls-files"]).splitlines()
  clean_hashes = {p: digest(REPO / p) for p in tracked}
  require(all((REPO / p).is_file() for p in SOURCE + ASSETS), "missing frozen assets")
  require(Path(__file__).resolve() == REPO / ASSETS[0], "helper must run from staged path")
  rustc = small(["rustup", "which", "--toolchain", "1.99.0", "rustc"])
  cargo = small(["rustup", "which", "--toolchain", "1.99.0", "cargo"])
  for k in tuple(ENV):
    if k.startswith("CARGO_") or k in ("RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
                    "RUSTC_BOOTSTRAP", "RUSTDOCFLAGS"):
      del ENV[k]
  ENV.update(CARGO_HOME=str(ROOT / "cargo-home"), CARGO_TARGET_DIR=str(ROOT / "target"),
       CARGO_INCREMENTAL="0", CARGO_BUILD_JOBS="2", CARGO_PROFILE_RELEASE_DEBUG="0",
       CARGO_PROFILE_RELEASE_CODEGEN_UNITS="1", CARGO_PROFILE_RELEASE_LTO="thin", RUSTFLAGS=FLAGS,
       RUSTUP_TOOLCHAIN="1.99.0", RUSTC=rustc)
  rustvv = small([rustc, "-Vv"])
  require(rustvv.startswith("rustc 1.99.0 "), "wrong Rust version")
  target = re.search(r"(?m)^host: (.+)$", rustvv).group(1)
  compiler_hashes = {rustc: digest(rustc), cargo: digest(cargo)}
  allowed = sorted(os.sched_getaffinity(0))
  require(allowed, "no allowed CPU")
  plan = json.loads(Path(ASSETS[2]).read_bytes())
  require(plan["baseline"] == BASE and plan["metrics"] == list(METRICS), "plan mismatch")
  original_manifest = Path("Cargo.toml").read_bytes()
  original_lock = Path("Cargo.lock").read_bytes()
  save("original-Cargo.toml", original_manifest)
  save("original-Cargo.lock", original_lock)
  for p in SOURCE + ASSETS:
    save("source/" + p, Path(p).read_bytes())
  original = {"clean_commit": commit, "clean_tree": tree, "clean_status": "",
        "baseline": BASE, "changed_paths": changed, "tracked_sha256": clean_hashes,
        "rustc_Vv": rustvv, "compiler_sha256": compiler_hashes,
        "target": target, "flags": FLAGS, "features": "std,oscore",
        "no_default_features": True, "environment": {k: ENV[k] for k in (
          "CARGO_HOME", "CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "CARGO_BUILD_JOBS",
          "CARGO_PROFILE_RELEASE_DEBUG", "CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "CARGO_PROFILE_RELEASE_LTO",
          "RUSTUP_TOOLCHAIN", "RUSTC", "RUSTFLAGS")}}
  original.update(affinity={"allowed": allowed, "benchmark_cpu": allowed[0]},
          host=list(os.uname()), plan_sha256=digest(ASSETS[2]))
  save("clean-inputs.json", original)  # BEFORE any injection or dependency resolution.
  baseline = small(["git", "show", BASE + ":src/oscore/context.rs"])
  context = Path(SOURCE[0]).read_text()
  old_lookup, old_open = method(baseline, "lookup")
  new_lookup, new_open = method(context, "lookup")
  require(old_lookup[old_open:] == new_lookup[new_open:], "public lookup differs from baseline")
  observe, _ = method(baseline, "observe_request")
  require("baseline_observe_request" not in context, "baseline already injected")
  current_observe, _ = method(context, "observe_request")
  anchor = context.index(current_observe) + len(current_observe)
  context = context[:anchor] + "\n\n    #[cfg(test)]\n" + observe.replace(
    "fn observe_request(", "fn baseline_observe_request(", 1) + context[anchor:]
  Path(SOURCE[0]).write_text(context)
  protect = Path(SOURCE[1])
  require("binding_cost_probe" not in protect.read_text(), "harness already injected")
  bp = small(["git", "show", BASE + ":src/oscore/protect.rs"])
  oracle, _ = method(bp, "unprotect_bound_response_wire")
  save("baseline-protect.rs", bp.encode())
  current, _ = method(protect.read_text(), "unprotect_bound_response_wire")
  oracle = oracle.replace("fn unprotect_bound_response_wire", "fn baseline_unprotect_bound_response_wire").replace(".observe_request(", ".baseline_observe_request(")
  protect.write_text(protect.read_text().replace(current, current + "\n\n    #[cfg(test)]\n" + oracle, 1) + '\n#[cfg(test)]\n#[path = "criterion_probe.rs"]\nmod binding_cost_probe;\n')
  harness = Path("src/oscore/criterion_probe.rs")
  require(not harness.exists(), "sibling harness already exists")
  shutil.copyfile(ASSETS[1], harness)
  manifest = original_manifest.decode()
  require(manifest.count("[dev-dependencies]") == 1 and "criterion" not in manifest,
      "unexpected existing dev dependency")
  Path("Cargo.toml").write_text(manifest.replace("[dev-dependencies]", "[dev-dependencies]\n"
    "criterion = { version = '=0.5.1', default-features = false }", 1))
  fixture_path = Path("src/app/tests.rs")
  fixture_blob = small(["git", "rev-parse", BASE + ":src/app/tests.rs"])
  require(small(["git", "hash-object", str(fixture_path)]) == fixture_blob,
      "existing app test fixtures differ from baseline")
  fixture_before = digest(fixture_path)
  fixture_text, fixture_old, fixture_new = explicit_empty_fixture_slices(fixture_path.read_text())
  fixture_path.write_text(fixture_text)
  save("injected/test-typing.json", {"path": str(fixture_path), "baseline_blob": fixture_blob,
    "before_sha256": fixture_before, "after_sha256": digest(fixture_path),
    "occurrences": 2, "old": fixture_old, "new": fixture_new,
    "scope": "temporary libtest type annotation only; assertions retained"})
  for p in (SOURCE[0], SOURCE[1], "Cargo.toml"):
    save("injected/" + p, Path(p).read_bytes())
  resource_guard()
  build = [cargo, "test", "--release", "--no-default-features", "--features", "std,oscore",
      "--lib", "--no-run", "--message-format=json"]
  raw = run(build, "build", 540, cap=640 * 1024, building=True)
  resource_guard()
  selected = Path("Cargo.lock").read_bytes()
  save("selected-Cargo.lock", selected)
  lock_guard(original_lock, selected)
  executables = set()
  for line in raw.splitlines():
    try:
      record = json.loads(line)
    except (ValueError, UnicodeError):
      continue
    if (isinstance(record, dict) and record.get("reason") == "compiler-artifact" and record.get("profile", {}).get("test")
        and record.get("target", {}).get("name") == "coaptic"
        and "lib" in record.get("target", {}).get("kind", []) and record.get("executable")):
      executables.add(record["executable"])
  require(len(executables) == 1, "Cargo did not report exactly one coaptic libtest executable")
  exe = Path(executables.pop()).resolve()
  require(exe.is_relative_to(ROOT / "target") and exe.is_file(), "executable outside owned target")
  identity_paths = set(tracked + [str(harness)])
  identity_paths.update((str(exe), rustc, cargo))
  frozen = {p: digest(p) for p in sorted(identity_paths)}
  save("frozen-inputs.json", {**original, "frozen_sha256": frozen, "build_command": build,
    "executable": str(exe), "executable_sha256": digest(exe),
    "manifest_sha256": digest("Cargo.toml"), "selected_lock_sha256": digest("Cargo.lock"),
    "test": TEST, "arms": ARMS, "parameters": {"samples": 30, "warmup_seconds": 0.5,
    "measurement_seconds": 1, "resamples": 10000, "confidence": 0.95},
    "limits": {"build_seconds": 540, "correctness_seconds": 120, "arm_seconds": 45,
    "build_log_bytes": 640 * 1024, "native_log_bytes": 65536, "artifact_bytes": ART_CAP,
    "criterion_bytes": RAW_CAP, "criterion_files": 256, "target_cargo_bytes": 8 * 1024**3}})
  def unchanged():
    require(frozen == {p: digest(p) for p in sorted(identity_paths)}, "frozen identity changed")
    require(small(["git", "rev-parse", "HEAD", "HEAD^{tree}"]) == commit + "\n" + tree,
        "checkout commit/tree changed")
  unchanged()
  proof = run([str(exe), "oscore::", "--nocapture", "--test-threads=1"], "correctness", 120)
  unchanged()
  require(re.search(rb"test result: ok\. [1-9][0-9]* passed; 0 failed;", proof) is not None,
      "no successful nonignored OSCORE tests")
  STATE["correctness"] = "PASS"
  fixtures = None
  for entry, (n, arm) in zip(STATE["arms"], ARMS):
    unchanged()
    arm_env = ENV.copy()
    arm_env.update(ROUND=str(n), ARM=arm,
           BINDING_PROBE_OUTPUT=str(ROOT / "criterion" / ("round" + str(n))))
    entry["status"] = "RUNNING"
    try:
      output = run([str(exe), "--ignored", "--exact", TEST, "--nocapture", "--test-threads=1"],
        "round" + str(n) + "-" + arm, 45, env=arm_env, cpu=allowed[0])
      unchanged()
      wires = sorted(re.findall(rb"BINDING_FIXTURE[^\r\n]*", output))
      require(len(wires) == 8 and (fixtures is None or wires == fixtures), "fixture mismatch")
      fixtures = wires
      entry["fixture_sha256"] = hashlib.sha256(b"\n".join(wires)).hexdigest()
      _, files = inventory(ROOT / "criterion", RAW_CAP, 256)  # Between arms only.
      for p in files:
        if p.suffix == ".json":
          save("criterion/" + str(p.relative_to(ROOT / "criterion")), p.read_bytes())
      entry["status"] = "PASS"
    except Exception:
      entry["status"] = "FAIL"
      raise
  decide()
  STATE["identities"] = "PASS"
if __name__ == "__main__":
  error = None
  try:
    main()
  except Exception as exc:
    error = type(exc).__name__ + ": " + str(exc)[:1500]
    STATE["status"] = "INCONCLUSIVE"
    STATE["failure"] = error
    if STATE["correctness"] == "NOTRUN" and (EVIDENCE / "correctness.stdout.log").exists():
      STATE["correctness"] = "FAIL"
  finally:
    if OWNED:
      try:
        _, files = inventory(ROOT / "criterion", RAW_CAP, 256)
        for p in files:
          if p.suffix == ".json":
            save("criterion/" + str(p.relative_to(ROOT / "criterion")), p.read_bytes())
        save("decision.json", STATE)
        if error:
          save("failure.json", {"error": error, "later_arms": STATE["arms"]})
        inventory(EVIDENCE, ART_CAP, 512)
      except Exception as exc:
        error = "evidence finalization: " + str(exc)[:1500]
        payload = (json.dumps({"status": "INCONCLUSIVE", "failure": error,
                   "arms": STATE["arms"]}) + "\n").encode()
        size, _ = inventory(EVIDENCE, ART_CAP, 512)
        if size + 2 * len(payload) <= ART_CAP:
          (EVIDENCE / "decision.json").write_bytes(payload)
          (EVIDENCE / "failure.json").write_bytes(payload)
        else:
          sys.stderr.write(error + "\n")
  if error:
    sys.stderr.write(error + "\n")
    sys.exit(1)
