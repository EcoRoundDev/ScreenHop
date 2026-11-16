#!/usr/bin/env python3
import argparse
import base64
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import shutil
import subprocess
import sys
import zipfile

ROOT = Path(__file__).resolve().parent.parent
POLICY = ROOT / "ci" / "macos-dev-signing-policy.json"


def _load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


signing = _load_module("screenhop_macos_signing", ROOT / "scripts" / "macos_signing.py")
dev_package = _load_module("screenhop_macos_dev_package", ROOT / "scripts" / "macos-dev.py")


def _execute(args, *, input_text=None, env=None):
    try:
        return subprocess.run([str(arg) for arg in args], check=False, capture_output=True, text=True, input=input_text, env=env)
    except (OSError, subprocess.SubprocessError):
        raise RuntimeError(f"无法执行 {Path(str(args[0])).name}") from None


def _run(args, *, input_text=None, env=None):
    result = _execute(args, input_text=input_text, env=env)
    if result.returncode:
        raise RuntimeError(f"{Path(str(args[0])).name} 执行失败（退出码 {result.returncode}）")
    return result.stdout.strip()


def _configured_policy(path):
    return signing.load_policy(path)


def _require_hosted():
    if os.environ.get("RUNNER_ENVIRONMENT") != "github-hosted":
        raise RuntimeError("仅允许 GitHub-hosted runner 执行此操作")
    if not os.environ.get("RUNNER_TEMP"):
        raise RuntimeError("缺少 RUNNER_TEMP")


def _attempt_root():
    _require_hosted()
    run_id = os.environ.get("GITHUB_RUN_ID", "")
    attempt = os.environ.get("GITHUB_RUN_ATTEMPT", "")
    if not re.fullmatch(r"[0-9]+", run_id) or not re.fullmatch(r"[0-9]+", attempt):
        raise RuntimeError("缺少有效的 workflow run ID/attempt")
    runner_temp = Path(os.environ["RUNNER_TEMP"]).resolve()
    base = runner_temp / "screenhop-macos-dev-signing"
    current = base / f"{run_id}-{attempt}"
    if base.is_symlink() or current.is_symlink():
        raise RuntimeError("拒绝访问符号链接临时目录")
    return base, current


def _private_dir(create=False):
    _, attempt = _attempt_root()
    path = attempt / "private"
    if path.is_symlink() or path.resolve().parent != attempt.resolve():
        raise RuntimeError("拒绝访问临时签名目录")
    if create:
        path.mkdir(mode=0o700, parents=True, exist_ok=False)
        path.chmod(0o700)
    return path


def preflight(policy_path=POLICY):
    _configured_policy(policy_path)
    _require_hosted()
    return True


def prepare(policy_path=POLICY, arm64_binary=None, x86_binary=None, build_version=None):
    preflight(policy_path)
    arm64_binary, x86_binary = Path(arm64_binary), Path(x86_binary)
    if not arm64_binary.is_file() or not x86_binary.is_file():
        raise RuntimeError("缺少已构建的架构二进制")
    base, attempt = _attempt_root()
    base.mkdir(mode=0o700, parents=True, exist_ok=True)
    try:
        attempt.mkdir(mode=0o700)
    except FileExistsError:
        raise RuntimeError("本次 workflow run attempt 已准备过，拒绝覆盖") from None
    bundles = attempt / "bundles"
    bundles.mkdir(mode=0o700)
    metadata = json.loads(_run(["cargo", "metadata", "--no-deps", "--format-version", "1", "--manifest-path", ROOT / "Cargo.toml"]))
    package = next(item for item in metadata["packages"] if item["name"] == "screenhop")
    build_version = build_version or signing.default_build_version()
    build_info_base = {
        "version": package["version"],
        "commit": os.environ.get("GITHUB_SHA", ""),
        "run_id": os.environ.get("GITHUB_RUN_ID", ""),
        "run_attempt": os.environ.get("GITHUB_RUN_ATTEMPT", ""),
    }
    targets = [
        ("arm64", arm64_binary, "aarch64-apple-darwin"),
        ("x86_64", x86_binary, "x86_64-apple-darwin"),
    ]
    universal_binary = bundles / "universal-screenhop"
    _run(["lipo", "-create", arm64_binary, x86_binary, "-output", universal_binary])
    targets.append(("universal", universal_binary, "universal"))
    outputs = {}
    for name, binary, target in targets:
        output = bundles / name / "ScreenHop Dev.app"
        output.parent.mkdir(mode=0o700)
        info = dict(build_info_base, target=target)
        dev_package.assemble_app(binary, output, build_version, info)
        outputs[name] = str(output)
    return outputs


def _extract_leaf(p12_path, password, private):
    pem = private / "leaf.pem"
    der = private / "leaf.der"
    _run(["openssl", "pkcs12", "-in", p12_path, "-clcerts", "-nokeys", "-out", pem, "-passin", "stdin"], input_text=password + "\n")
    _run(["openssl", "x509", "-in", pem, "-outform", "DER", "-out", der])
    pem.chmod(0o600)
    der.chmod(0o600)
    return pem, der


def _save_state(private, state):
    temporary = private / "state.json.tmp"
    temporary.write_text(json.dumps(state, sort_keys=True), encoding="utf-8")
    temporary.chmod(0o600)
    temporary.replace(private / "state.json")
    (private / "state.json").chmod(0o600)


def _load_state(private):
    path = private / "state.json"
    if not path.exists():
        return None
    if path.is_symlink():
        raise RuntimeError("拒绝读取无效临时签名状态")
    return json.loads(path.read_text(encoding="utf-8"))


def _identity_from_keychain(keychain, expected_sha1):
    text = _run(["security", "find-identity", "-v", "-p", "codesigning", keychain])
    identities = re.findall(r'^\s*\d+\)\s+([0-9a-fA-F]{40})\s+"', text, re.MULTILINE)
    matches = [item.lower() for item in identities if item.lower() == expected_sha1]
    if len(matches) != 1:
        raise RuntimeError("临时钥匙串中未找到唯一的策略证书身份")
    return matches[0]


def _sign(policy_path):
    policy = _configured_policy(policy_path)
    _require_hosted()
    encoded = os.environ.get("MACOS_DEV_CERT_P12_BASE64")
    password = os.environ.get("MACOS_DEV_CERT_PASSWORD")
    if not encoded or not password:
        raise RuntimeError("缺少签名证书 Secret；不提供 ad-hoc 回退")
    try:
        p12 = base64.b64decode(encoded, validate=True)
    except (ValueError, base64.binascii.Error):
        raise RuntimeError("PKCS#12 Secret 不是有效的 Base64") from None
    private = _private_dir(create=True)
    p12_path = private / "identity.p12"
    descriptor = os.open(p12_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(descriptor, "wb") as file:
        file.write(p12)
    leaf_pem, leaf_der = _extract_leaf(p12_path, password, private)
    der = leaf_der.read_bytes()
    if hashlib.sha1(der).hexdigest() != policy["certificate_sha1"] or hashlib.sha256(der).hexdigest() != policy["certificate_sha256"]:
        raise RuntimeError("PKCS#12 叶证书与固定策略指纹不匹配")
    keychain = private / "macos-dev.keychain-db"
    keychain_password = secrets.token_urlsafe(32)
    print(f"::add-mask::{keychain_password}")
    state = {"keychain": str(keychain), "leaf_der": str(leaf_der), "trust_attempted": False}
    _save_state(private, state)
    _run(["security", "create-keychain", "-p", keychain_password, keychain])
    _run(["security", "set-keychain-settings", "-lut", "21600", keychain])
    _run(["security", "unlock-keychain", "-p", keychain_password, keychain])
    _run(["security", "import", p12_path, "-k", keychain, "-P", password, "-T", "/usr/bin/codesign", "-T", "/usr/bin/security"])
    _run(["security", "set-key-partition-list", "-S", "apple-tool:,apple:,codesign:", "-s", "-k", keychain_password, keychain])
    if policy["certificate_kind"] == "self-signed":
        state["trust_attempted"] = True
        _save_state(private, state)
        _run(["security", "add-trusted-cert", "-r", "trustRoot", "-p", "codeSign", "-k", keychain, leaf_der])
    else:
        _run(["security", "verify-cert", "-c", leaf_pem, "-p", "codeSign", "-k", keychain])
    identity = _identity_from_keychain(keychain, policy["certificate_sha1"])
    _, attempt = _attempt_root()
    reports = {}
    for name, expected in (("arm64", ["arm64"]), ("x86_64", ["x86_64"]), ("universal", ["arm64", "x86_64"])):
        app = attempt / "bundles" / name / "ScreenHop Dev.app"
        try:
            _run(["codesign", "--force", "--sign", identity, "--keychain", keychain, app])
        except Exception:
            raise RuntimeError("codesign 签名失败") from None
        reports[name] = signing.verify_app(app, policy, expected_arches=expected)
    reports_path = attempt / "signing-reports.json"
    reports_path.write_text(json.dumps(reports, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    reports_path.chmod(0o600)
    return reports


def sign(policy_path=POLICY):
    try:
        return _sign(policy_path)
    finally:
        if os.environ.get("RUNNER_ENVIRONMENT") == "github-hosted" and os.environ.get("RUNNER_TEMP"):
            cleanup()


def _cleanup_run(args):
    result = _execute(args)
    if result.returncode:
        combined = (result.stdout + result.stderr).lower()
        if any(text in combined for text in ("not found", "no such", "not trusted", "unable to find", "the specified item could not be found in the keychain")):
            return
        raise RuntimeError(f"{Path(str(args[0])).name} 清理失败（退出码 {result.returncode}）")


def cleanup():
    _require_hosted()
    private = _private_dir()
    if not private.exists():
        return True
    failures = []
    try:
        state = _load_state(private)
    except (OSError, ValueError, RuntimeError):
        state = None
        failures.append("无法读取临时签名清理状态")
    if state and state.get("trust_attempted"):
        leaf = Path(state.get("leaf_der", ""))
        if leaf != private / "leaf.der" or leaf.is_symlink():
            failures.append("拒绝访问无效的受信证书路径")
        else:
            try:
                if not leaf.is_file():
                    raise RuntimeError("缺少撤销信任所需的叶证书")
                _cleanup_run(["security", "remove-trusted-cert", leaf])
            except RuntimeError as error:
                failures.append(str(error))
    if state and state.get("keychain"):
        keychain = Path(state["keychain"])
        if keychain != private / "macos-dev.keychain-db" or keychain.is_symlink():
            failures.append("拒绝删除临时目录外的钥匙串")
        elif keychain.exists():
            try:
                _cleanup_run(["security", "delete-keychain", keychain])
            except RuntimeError as error:
                failures.append(str(error))
    for name in ("identity.p12", "leaf.pem", "state.json.tmp"):
        path = private / name
        if path.is_symlink():
            failures.append("拒绝删除临时签名目录中的符号链接")
        elif path.exists():
            path.unlink()
    if failures:
        raise RuntimeError("; ".join(failures))
    for name in ("leaf.der", "state.json"):
        path = private / name
        if path.is_symlink():
            raise RuntimeError("拒绝删除临时签名目录中的符号链接")
        path.unlink(missing_ok=True)
    try:
        private.rmdir()
    except OSError:
        pass
    return True


def _archive_one(app, output, policy, expected, recorded=None):
    before = signing.verify_app(app, policy, expected_arches=expected)
    if recorded is not None and before != recorded:
        raise RuntimeError("归档前签名报告与签名步骤记录不同")
    if output.exists() or output.is_symlink():
        raise RuntimeError("拒绝覆盖已存在归档")
    extracted = output.parent / (output.stem + ".extracted")
    if extracted.exists() or extracted.is_symlink():
        raise RuntimeError("拒绝覆盖已存在的解包校验目录")
    extracted.mkdir(mode=0o700)
    try:
        _run(["ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, output])
        if not zipfile.is_zipfile(output):
            raise RuntimeError("ditto 未生成有效 ZIP")
        _run(["ditto", "-x", "-k", output, extracted])
        after = signing.verify_app(extracted / "ScreenHop Dev.app", policy, expected_arches=expected)
        if before != after:
            raise RuntimeError("ZIP 往返后的签名报告发生变化")
    except Exception:
        output.unlink(missing_ok=True)
        raise
    finally:
        shutil.rmtree(extracted, ignore_errors=True)
    return {"file": output.name, "sha256": hashlib.sha256(output.read_bytes()).hexdigest(), "verification": before}


def archive(policy_path=POLICY, output_dir=None):
    policy = _configured_policy(policy_path)
    preflight(policy_path)
    _, attempt = _attempt_root()
    private = _private_dir()
    if private.exists():
        raise RuntimeError("临时签名材料尚未清理，拒绝归档")
    reports_path = attempt / "signing-reports.json"
    if not reports_path.is_file() or reports_path.is_symlink():
        raise RuntimeError("缺少成功签名报告，拒绝归档")
    reports = json.loads(reports_path.read_text(encoding="utf-8"))
    if set(reports) != {"arm64", "x86_64", "universal"}:
        raise RuntimeError("签名报告不完整，拒绝归档")
    run_id, run_attempt = os.environ["GITHUB_RUN_ID"], os.environ["GITHUB_RUN_ATTEMPT"]
    short_sha = os.environ.get("GITHUB_SHA", "")[:12]
    if not re.fullmatch(r"[0-9a-fA-F]{7,40}", short_sha):
        raise RuntimeError("缺少有效的 commit SHA")
    destination = Path(output_dir or (Path(os.environ.get("GITHUB_WORKSPACE", ROOT)) / "artifacts")).resolve()
    destination.mkdir(parents=True, exist_ok=True)
    names = {
        "arm64": ("ScreenHop-Dev-macOS-arm64", ["arm64"]),
        "x86_64": ("ScreenHop-Dev-macOS-x86_64", ["x86_64"]),
        "universal": ("ScreenHop-Dev-macOS-universal", ["arm64", "x86_64"]),
    }
    files = []
    for name, (stem, expected) in names.items():
        report = _archive_one(attempt / "bundles" / name / "ScreenHop Dev.app", destination / f"{stem}-{short_sha}-{run_id}-attempt{run_attempt}.zip", policy, expected, reports[name])
        files.append(report)
    manifest = {
        "schema_version": 1,
        "commit": os.environ["GITHUB_SHA"],
        "run_id": run_id,
        "run_attempt": run_attempt,
        "build_version": reports["arm64"]["build_version"],
        "bundle_id": signing.BUNDLE_ID,
        "certificate_sha1": policy["certificate_sha1"],
        "certificate_sha256": policy["certificate_sha256"],
        "certificate_kind": policy["certificate_kind"],
        "designated_requirement": policy["designated_requirement"],
        "artifacts": files,
    }
    manifest_path, sums_path = destination / "signing-manifest.json", destination / "SHA256SUMS"
    if manifest_path.exists() or sums_path.exists():
        raise RuntimeError("拒绝覆盖已存在的签名清单")
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    manifest_path.chmod(0o644)
    sums_path.write_text("".join(f"{item['sha256']}  {item['file']}\n" for item in files), encoding="utf-8")
    sums_path.chmod(0o644)
    return [str(destination / item["file"]) for item in files] + [str(manifest_path), str(sums_path)]


def main():
    parser = argparse.ArgumentParser(description="GitHub-hosted macOS Dev 固定身份签名和归档")
    commands = parser.add_subparsers(dest="command", required=True)
    for command in ("preflight", "sign", "archive"):
        child = commands.add_parser(command)
        child.add_argument("--policy", type=Path, default=POLICY)
        if command == "archive":
            child.add_argument("--output-dir", type=Path)
    build = commands.add_parser("prepare")
    build.add_argument("--policy", type=Path, default=POLICY)
    build.add_argument("--arm64-binary", type=Path, required=True)
    build.add_argument("--x86-binary", type=Path, required=True)
    build.add_argument("--build-version")
    commands.add_parser("cleanup")
    args = parser.parse_args()
    try:
        if args.command == "preflight":
            preflight(args.policy)
        elif args.command == "prepare":
            prepare(args.policy, args.arm64_binary, args.x86_binary, args.build_version)
        elif args.command == "sign":
            sign(args.policy)
        elif args.command == "archive":
            archive(args.policy, args.output_dir)
        else:
            cleanup()
        return 0
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError, StopIteration) as error:
        print(f"macOS Dev CI 失败：{error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
