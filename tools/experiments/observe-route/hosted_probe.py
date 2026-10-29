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
BASE = "176c337d350a4f601e4c2a02b9bdfdcaf0792a92"
CANDIDATE = "2349287f0bf78212125d94ab7316e4bcf604c326"
TEST = "app::observe_route_probe::observe_route_criterion_probe"
METRICS = ("routes4-first-unprotected", "routes4-last-unprotected",
           "routes32-first-unprotected", "routes32-last-unprotected",
           "routes4-first-oscore", "routes4-last-oscore",
           "routes32-first-oscore", "routes32-last-oscore")
BENEFITS = ("routes32-last-unprotected", "routes32-last-oscore")
GUARDS = tuple(m for m in METRICS if m not in BENEFITS)
ARMS = ((1, "baseline"), (1, "candidate"), (2, "candidate"), (2, "baseline"))
# Cargo selects LTO per crate type; global -C lto rejects proc-macro dependencies.
FLAGS = ""
MIB = 1024 * 1024
ART_CAP = 3 * MIB
RAW_CAP = 512 * 1024
SOURCE = ("src/app/site.rs", "src/storage/table.rs")
DOCS = ("PLAN.md", "architecture/requirements/OPTIMIZATION-ITERATIONS.md")
ASSETS = ("tools/experiments/observe-route/hosted_probe.py",
     "tools/experiments/observe-route/criterion_probe.rs",
     "tools/experiments/observe-route/measurement-plan.json", ".github/workflows/ci.yml")
ROOT = Path(os.environ["RUNNER_TEMP"]).resolve() / "observe-route-probe"
EVIDENCE = ROOT / "evidence"
REPO = Path.cwd().resolve()
ENV = os.environ.copy()
STATE = {"status": "INCONCLUSIVE", "correctness": "NOTRUN", "arms": [
  {"round": n, "arm": arm, "status": "NOTRUN"} for n, arm in ARMS],
  "fixture_parity": "NOTRUN", "fixtures": [
    {"arm": arm, "status": "NOTRUN"} for arm in ("baseline", "candidate")]}
OWNED = False
META = 0
JOB_DEADLINE = None
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
  if JOB_DEADLINE is not None:
    deadline = min(deadline, JOB_DEADLINE - 10)  # leave bounded process cleanup time
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
def bounded_span(blob, anchor, following, size, sha256):
  # git-show bytes are canonical LF. Never strip text or parse Rust braces:
  # the pinned engine contains literals, lifetimes, macros and comments.
  require(b"\r" not in blob, "noncanonical source line endings")
  require(blob.count(anchor) == blob.count(following) == 1, "ambiguous source anchors")
  start = blob.index(anchor)
  end = blob.index(following)
  require(end - start == size, "pinned source span size changed")
  span = blob[start:end]
  require(hashlib.sha256(span).hexdigest() == sha256, "pinned source span hash changed")
  return span

def replace_once(blob, old, new):
  require(blob.count(old) == 1, "unexpected source replacement count: " + repr(old))
  return blob.replace(old, new, 1)

def oracle_sources(app, site, current_app, current_site):
  poll = bounded_span(app, b"    pub fn poll(",
      b"    /// Poll with caller-owned stateful application dispatch and optional deferral.\n",
      591, "dee35021506f7519a06685e95303af370677bce7c83ce4d999924f1b57436c22")
  engine = bounded_span(app, b"fn poll_engine<\n",
      b"// Only an exact retained CON packet can advance a Q Observe representation.\n",
      7997, "7804bca9109868a4474c9491e2be6ff2a042b19ac6bb2a8516a53b7920703e13")
  scan = bounded_span(site, b"    pub(crate) fn observe_source(",
      b"}\n\nimpl<const N: usize, T: OwnedToken> Default for Site<N, T> {\n",
      368, "52031b2030bd850fa5302d27c78fd9ab5825b2c8602c7ecef384976aeb350169")
  require(current_app == app, "App source differs from unchanged baseline")
  direct = (b"    pub(crate) fn observe_source(&self, resource: ObserveResource) -> Option<SnapshotSource<T>> {\n"
      b"        self.entries\n"
      b"            .get(resource.app_route_index()?)?\n"
      b"            .as_ref()?\n"
      b"            .methods\n"
      b"            .observe_source()\n"
      b"    }\n")
  require(current_site.count(direct) == 1, "candidate Site method shape changed")
  require(current_app.count(poll) == current_app.count(engine) == 1,
      "original poll/engine span not unique")
  attributes = app[:app.index(engine)].splitlines(keepends=True)[-2:]
  require(len(attributes) == 2 and attributes[0] == b"#[allow(clippy::too_many_arguments)]\n"
      and attributes[1].startswith(b"#[coaptic_io::dual_io(poll_engine_async,")
      and attributes[1].endswith(b")]\n"), "unexpected original engine attributes")
  poll_copy = replace_once(poll, b"fn poll(", b"fn poll_scan_oracle(")
  poll_copy = replace_once(poll_copy, b"        poll_engine(\n", b"        poll_engine_scan_oracle(\n")
  engine_copy = replace_once(engine, b"fn poll_engine<\n", b"fn poll_engine_scan_oracle<\n")
  due_call = b"site.observe_source(interest.resource())"
  scan_call = b"site.observe_source_scan_oracle(interest.resource())"
  engine_copy = replace_once(engine_copy, due_call, scan_call)
  scan_copy = replace_once(scan, b"fn observe_source(", b"fn observe_source_scan_oracle(")
  restored_poll = replace_once(poll_copy, b"fn poll_scan_oracle(", b"fn poll(")
  restored_poll = replace_once(restored_poll, b"        poll_engine_scan_oracle(\n", b"        poll_engine(\n")
  restored_engine = replace_once(engine_copy, b"fn poll_engine_scan_oracle<\n", b"fn poll_engine<\n")
  restored_engine = replace_once(restored_engine, scan_call, due_call)
  restored_scan = replace_once(scan_copy, b"fn observe_source_scan_oracle(", b"fn observe_source(")
  require((restored_poll, restored_engine, restored_scan) == (poll, engine, scan),
      "inverse-renamed oracle differs from baseline")
  poll_addition = b"    #[cfg(test)]\n" + poll_copy
  root_addition = (b"\n#[cfg(test)]\n" + attributes[0] + engine_copy
      + b'#[cfg(all(test, feature = "std", feature = "oscore"))]\n'
        b'#[path = "observe_route_probe.rs"]\nmod observe_route_probe;\n')
  site_addition = b"\n    #[cfg(test)]\n" + scan_copy
  injected_app = replace_once(current_app, poll, poll + poll_addition) + root_addition
  injected_site = replace_once(current_site, direct, direct + site_addition)
  require(replace_once(injected_app[:-len(root_addition)], poll_addition, b"") == current_app
      and replace_once(injected_site, site_addition, b"") == current_site,
      "injection changed original production source")
  copies = {"poll": (poll, poll_copy), "engine": (engine, engine_copy), "site": (scan, scan_copy)}
  receipt = {"inverse_renamed_equal": True, "original_production_bytes_preserved": True,
      "baseline_lines_inclusive": {"poll": [2132, 2153], "engine": [2662, 2875], "site": [390, 397]},
      "engine_original_attributes": b"".join(attributes).decode(),
      "engine_copy_attributes": (b"#[cfg(test)]\n" + attributes[0]).decode(),
      "omitted_attribute": "coaptic_io::dual_io only; original sync/async production source unchanged",
      "replacement_counts": {"poll_declaration": 1, "poll_engine_call": 1,
        "engine_declaration": 1, "due_site_call": 1, "site_declaration": 1},
      "copies": {name: {"baseline_bytes": len(old), "baseline_sha256": hashlib.sha256(old).hexdigest(),
          "copy_bytes": len(new), "copy_sha256": hashlib.sha256(new).hexdigest()}
        for name, (old, new) in copies.items()}}
  return injected_app, injected_site, copies, receipt
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
         "                    &[]\n")
  require(text.count(old) == 2, "unexpected empty-slice fixture sites")
  new = old[:-1] + " as &[u8]\n"
  return text.replace(old, new), old, new

def fixture_lines(output):
  # libtest may put the first marker after its "test ..." prefix.
  return sorted(re.findall(rb"OBSERVE_ROUTE_FIXTURE[^\r\n]*", output))

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
        require(all(finite(v) > 0 for v in sample["iters"] + sample["times"]),
            "nonpositive Criterion sample")
        mean = interval(read(saved + "/estimates.json")["mean"])
        require(mean["lower"] > 0, "nonpositive measured mean")
        estimates[saved] = mean
        read(saved + "/benchmark.json")
      row[metric] = {"candidate_vs_baseline": change, "raw_mean_ns": estimates,
             "first_arm": "baseline" if n == 1 else "candidate"}
    result[str(n)] = row
  accept = all(
    all(row[m]["candidate_vs_baseline"]["point"] <= -0.05
        and row[m]["candidate_vs_baseline"]["upper"] < 0 for m in BENEFITS)
    and all(row[m]["candidate_vs_baseline"]["upper"] <= 0.02 for m in GUARDS)
    for row in result.values())
  STATE.update(status="ACCEPT" if accept else "INCONCLUSIVE/REJECT", rounds=result,
        scope="instrumented hosted complete-notification poll wall cost only; no replay-admission, socket, CPU50, emitted-stack, whole-memory or physical-device claim")
def main():
  global OWNED, JOB_DEADLINE
  require(sys.platform == "linux", "hosted Linux only")
  # Leave one minute of the 15-minute job for checkout and artifact publication.
  JOB_DEADLINE = time.monotonic() + 840
  require(not ROOT.exists(), "owned root already exists; no retries")
  EVIDENCE.mkdir(parents=True)
  OWNED = True
  require(not small(["git", "status", "--porcelain", "--untracked-files=all"]),
      "checkout is not clean before injection")
  commit = small(["git", "rev-parse", "HEAD"])
  tree = small(["git", "rev-parse", "HEAD^{tree}"])
  require(re.fullmatch(r"[0-9a-f]{40}", commit) is not None, "invalid commit")
  changed = small(["git", "diff", "--name-only", BASE, "HEAD"]).splitlines()
  require(set(changed) <= set(SOURCE + ASSETS + DOCS), "production/temporary diff outside whitelist")
  tracked = small(["git", "ls-files"]).splitlines()
  clean_hashes = {p: digest(REPO / p) for p in tracked}
  require(all((REPO / p).is_file() for p in SOURCE + ASSETS), "missing frozen assets")
  require(Path(__file__).resolve() == REPO / ASSETS[0], "helper must run from staged path")
  rustc = small(["rustup", "which", "--toolchain", "1.99.0", "rustc"])
  cargo = small(["rustup", "which", "--toolchain", "1.99.0", "cargo"])
  for k in tuple(ENV):
    if k.startswith("CARGO_") or k in ("RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
                    "RUSTC_BOOTSTRAP", "RUSTDOCFLAGS", "RUST_MIN_STACK",
                    "OBSERVE_ROUTE_FIXTURES_ONLY", "ROUND", "ARM", "BINDING_PROBE_OUTPUT"):
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
  require(plan["benefit_metrics"] == list(BENEFITS)
      and plan["arms"] == [list(arm) for arm in ARMS], "plan gates/arms mismatch")
  for key, expected in {"samples": 30, "warmup_seconds": 0.2, "measurement_seconds": 0.5,
      "resamples": 10000, "confidence_level": 0.95, "rust": "1.99.0", "criterion": "0.5.1",
      "features": "std,oscore", "no_default_features": True,
      "release_lto": "thin", "release_codegen_units": 1}.items():
    require(plan["parameters"][key] == expected, "plan parameter mismatch: " + key)
  for key, expected in {"job_minutes": 15, "build_seconds": 540, "correctness_seconds": 120,
      "fixture_process_seconds": 60,
      "measurement_total_seconds": 240, "arm_seconds_max": 90,
      "target_plus_cargo_home_bytes": 8 * 1024**3, "free_bytes_min": 4 * 1024**3,
      "remote_evidence_bytes": ART_CAP, "criterion_bytes": RAW_CAP}.items():
    require(plan["limits"][key] == expected, "plan limit mismatch: " + key)
  original_manifest = Path("Cargo.toml").read_bytes()
  original_lock = Path("Cargo.lock").read_bytes()
  save("original-Cargo.toml", original_manifest)
  save("original-Cargo.lock", original_lock)
  for p in SOURCE + ASSETS:
    save("source/" + p, Path(p).read_bytes())
  original = {"clean_commit": commit, "clean_tree": tree, "clean_status": "",
        "baseline": BASE, "candidate_source_commit": CANDIDATE,
        "changed_paths": changed, "tracked_sha256": clean_hashes,
        "rustc_Vv": rustvv, "compiler_sha256": compiler_hashes,
        "target": target, "flags": FLAGS, "features": "std,oscore",
        "no_default_features": True, "environment": {k: ENV[k] for k in (
          "CARGO_HOME", "CARGO_TARGET_DIR", "CARGO_INCREMENTAL", "CARGO_BUILD_JOBS",
          "CARGO_PROFILE_RELEASE_DEBUG", "CARGO_PROFILE_RELEASE_CODEGEN_UNITS", "CARGO_PROFILE_RELEASE_LTO",
          "RUSTUP_TOOLCHAIN", "RUSTC", "RUSTFLAGS")}}
  original.update(affinity={"allowed": allowed, "benchmark_cpu": allowed[0],
          "cpu0_available": 0 in allowed, "selection": "first actual allowed CPU"},
          host=list(os.uname()), plan_sha256=digest(ASSETS[2]))
  cpu_records = [dict(line.split(":", 1) for line in block.splitlines() if ":" in line)
      for block in Path("/proc/cpuinfo").read_text().split("\n\n")]
  cpu_records = [{key.strip(): value.strip() for key, value in row.items()} for row in cpu_records]
  original["benchmark_cpu_info"] = next(
      (row for row in cpu_records if row.get("processor") == str(allowed[0])), None)
  require(original["benchmark_cpu_info"] is not None, "selected CPU metadata missing")
  save("clean-inputs.json", original)  # BEFORE any injection or dependency resolution.
  # Refuse any candidate drift, including tests. Only the approved two-file
  # candidate is production input; all oracle/harness edits live in this job.
  for path in SOURCE:
    require(small(["git", "hash-object", path]) ==
        small(["git", "rev-parse", CANDIDATE + ":" + path]), "approved candidate source changed: " + path)
  app_path = Path("src/app/mod.rs")
  site_path = Path(SOURCE[0])
  baseline_app = run(["git", "show", BASE + ":src/app/mod.rs"], "baseline-app-source", 15, cap=384 * 1024)
  baseline_site = run(["git", "show", BASE + ":src/app/site.rs"], "baseline-site-source", 15)
  current_app = app_path.read_bytes()
  current_site = site_path.read_bytes()
  require(b"observe_route_probe" not in current_app and b"scan_oracle" not in current_app
      and b"scan_oracle" not in current_site, "oracle/harness already injected")
  injected_app, injected_site, copies, oracle_receipt = oracle_sources(
      baseline_app, baseline_site, current_app, current_site)
  for name, (old, new) in copies.items():
    save("oracle/" + name + "-baseline.rs", old)
    save("oracle/" + name + "-copy.rs", new)
  save("oracle/receipt.json", oracle_receipt)
  app_path.write_bytes(injected_app)
  site_path.write_bytes(injected_site)
  harness = Path("src/app/observe_route_probe.rs")
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
  for p in (str(app_path), SOURCE[0], SOURCE[1], str(harness), "Cargo.toml"):
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
    "test": TEST, "arms": ARMS, "parameters": {"samples": 30, "warmup_seconds": 0.2,
    "measurement_seconds": 0.5, "resamples": 10000, "confidence": 0.95},
    "oracle": oracle_receipt,
    "limits": {"build_seconds": 540, "correctness_seconds": 120, "arm_seconds_max": 90,
    "fixture_seconds_max": 60, "fixture_total_seconds_max": 120,
    "measurement_total_seconds": 240, "runner_seconds_max": 840,
    "test_stack": "unchanged default; RUST_MIN_STACK removed",
    "build_log_bytes": 640 * 1024, "correctness_log_bytes": 256 * 1024,
    "native_log_bytes": 65536, "artifact_bytes": ART_CAP,
    "criterion_bytes": RAW_CAP, "criterion_files": 512, "target_cargo_bytes": 8 * 1024**3}})
  def unchanged():
    require(frozen == {p: digest(p) for p in sorted(identity_paths)}, "frozen identity changed")
    require(small(["git", "rev-parse", "HEAD", "HEAD^{tree}"]) == commit + "\n" + tree,
        "checkout commit/tree changed")
  unchanged()
  # All nonignored library tests include the new Site and table slot tests.
  proof = run([str(exe), "--nocapture", "--test-threads=1"], "correctness", 120, cap=256 * 1024)
  unchanged()
  require(re.search(rb"test result: ok\. [1-9][0-9]* passed; 0 failed;", proof) is not None,
      "no successful nonignored library tests")
  STATE["correctness"] = "PASS"
  fixtures = None
  fixture_started = time.monotonic()
  fixture_deadline = min(fixture_started + 120, JOB_DEADLINE)
  STATE["fixture_parity"] = "RUNNING"
  for entry in STATE["fixtures"]:
    unchanged()
    fixture_env = ENV.copy()
    fixture_env.update(ROUND="1", ARM=entry["arm"], OBSERVE_ROUTE_FIXTURES_ONLY="1",
        BINDING_PROBE_OUTPUT=str(ROOT / "criterion" / "fixture-check"))
    entry["status"] = "RUNNING"
    started = time.monotonic()
    try:
      remaining = fixture_deadline - time.monotonic()
      require(remaining > 10, "fixture-only aggregate deadline exhausted")
      output = run([str(exe), "--ignored", "--exact", TEST, "--nocapture", "--test-threads=1"],
          "fixture-" + entry["arm"], min(50, remaining - 10), env=fixture_env, cpu=allowed[0])
      entry["elapsed_seconds"] = time.monotonic() - started
      require(entry["elapsed_seconds"] <= 60 and time.monotonic() < fixture_deadline,
          "fixture-only process/aggregate overrun")
      unchanged()
      wires = fixture_lines(output)
      require(len(wires) == len(set(wires)) == 8 and (fixtures is None or wires == fixtures),
          "premeasurement fixture count/identity mismatch")
      fixtures = wires
      entry["fixture_sha256"] = hashlib.sha256(b"\n".join(wires)).hexdigest()
      _, files = inventory(ROOT / "criterion", RAW_CAP, 512)
      require(not files, "fixture-only process emitted measurement files")
      entry["status"] = "PASS"
    except Exception as exc:
      entry["status"] = "FAIL"
      entry["elapsed_seconds"] = time.monotonic() - started
      entry["failure"] = type(exc).__name__ + ": " + str(exc)[:1500]
      STATE["fixture_parity"] = "FAIL"
      raise
  require(fixtures is not None, "no premeasurement fixture receipts")
  STATE["fixture_parity"] = "PASS"
  STATE["fixture_elapsed_seconds"] = time.monotonic() - fixture_started
  save("fixture-parity.json", {"status": "PASS", "processes": STATE["fixtures"],
      "fixture_lines": [line.decode() for line in fixtures],
      "fixture_sha256": hashlib.sha256(b"\n".join(fixtures)).hexdigest(),
      "scope": "two distinct isolated baseline/candidate processes before any sampling"})
  measurement_started = time.monotonic()
  measurement_deadline = min(measurement_started + 240, JOB_DEADLINE)
  for entry, (n, arm) in zip(STATE["arms"], ARMS):
    unchanged()
    arm_env = ENV.copy()
    arm_env.update(ROUND=str(n), ARM=arm,
           BINDING_PROBE_OUTPUT=str(ROOT / "criterion" / ("round" + str(n))))
    entry["status"] = "RUNNING"
    arm_started = time.monotonic()
    try:
      # Reserve the reviewed five-second wait and five-second settlement bound
      # within the aggregate budget, including failure cleanup.
      remaining = measurement_deadline - time.monotonic()
      require(remaining > 10, "aggregate measurement deadline exhausted")
      arm_timeout = min(80, remaining - 10)
      entry["process_timeout_seconds"] = arm_timeout
      output = run([str(exe), "--ignored", "--exact", TEST, "--nocapture", "--test-threads=1"],
        "round" + str(n) + "-" + arm, arm_timeout, env=arm_env, cpu=allowed[0])
      entry["elapsed_seconds"] = time.monotonic() - arm_started
      require(entry["elapsed_seconds"] <= 90 and time.monotonic() < measurement_deadline,
          "arm/aggregate measurement overrun")
      unchanged()
      wires = fixture_lines(output)
      require(len(wires) == len(set(wires)) == 8 and wires == fixtures,
          "timing-arm fixture count/identity mismatch")
      entry["fixture_lines_emitted"] = len(wires)
      entry["fixture_sha256"] = hashlib.sha256(b"\n".join(fixtures)).hexdigest()
      _, files = inventory(ROOT / "criterion", RAW_CAP, 512)  # Between arms only.
      for p in files:
        if p.suffix == ".json":
          save("criterion/" + str(p.relative_to(ROOT / "criterion")), p.read_bytes())
      entry["status"] = "PASS"
    except Exception as exc:
      entry["status"] = "FAIL"
      entry["elapsed_seconds"] = time.monotonic() - arm_started
      entry["failure"] = type(exc).__name__ + ": " + str(exc)[:1500]
      raise
  STATE["measurement_elapsed_seconds"] = time.monotonic() - measurement_started
  require(STATE["measurement_elapsed_seconds"] <= 240, "aggregate measurement overrun")
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
        _, files = inventory(ROOT / "criterion", RAW_CAP, 512)
        for p in files:
          if p.suffix == ".json":
            save("criterion/" + str(p.relative_to(ROOT / "criterion")), p.read_bytes())
        save("decision.json", STATE)
        if error:
          save("failure.json", {"error": error, "later_arms": STATE["arms"],
              "fixture_parity": STATE["fixture_parity"], "fixtures": STATE["fixtures"]})
        inventory(EVIDENCE, ART_CAP, 512)
      except Exception as exc:
        error = "evidence finalization: " + str(exc)[:1500]
        payload = (json.dumps({"status": "INCONCLUSIVE", "failure": error,
                   "arms": STATE["arms"], "fixture_parity": STATE["fixture_parity"],
                   "fixtures": STATE["fixtures"]}) + "\n").encode()
        size, _ = inventory(EVIDENCE, ART_CAP, 512)
        if size + 2 * len(payload) <= ART_CAP:
          (EVIDENCE / "decision.json").write_bytes(payload)
          (EVIDENCE / "failure.json").write_bytes(payload)
        else:
          sys.stderr.write(error + "\n")
  if error:
    sys.stderr.write(error + "\n")
    sys.exit(1)
