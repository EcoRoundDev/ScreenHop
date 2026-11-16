import argparse
import hashlib
import json
from pathlib import Path
import plistlib
import platform
import re
import subprocess
import tempfile

APP_NAME = "ScreenHop Dev"
BUNDLE_ID = "com.ecorounddev.screenhop.dev"
DEFAULT_POLICY = Path(__file__).resolve().parent.parent / "ci" / "macos-dev-signing-policy.json"


def run(args, **kwargs):
    return subprocess.run([str(arg) for arg in args], check=True, **kwargs)


def output(args):
    return run(args, capture_output=True, text=True).stdout.strip()


def normalized_fingerprint(value):
    return re.sub(r"[^0-9a-fA-F]", "", value).lower()


def load_policy(path=DEFAULT_POLICY):
    with Path(path).open(encoding="utf-8") as file:
        policy = json.load(file)
    if not isinstance(policy, dict) or policy.get("schema_version") != 1:
        raise RuntimeError("不支持的 macOS Dev 签名策略版本")
    if policy.get("bundle_id") != BUNDLE_ID or policy.get("app_name") != APP_NAME:
        raise RuntimeError("macOS Dev 签名策略的应用标识不匹配")
    if policy.get("configured") is not True:
        raise RuntimeError("macOS Dev 签名策略尚未配置可信证书基线")
    if policy.get("certificate_kind") not in ("apple-development", "self-signed"):
        raise RuntimeError("签名策略的证书类型无效")
    for key, length in (("certificate_sha1", 40), ("certificate_sha256", 64)):
        raw = policy.get(key)
        if not isinstance(raw, str) or not re.fullmatch(r"[0-9a-fA-F:\s]+", raw):
            raise RuntimeError("签名策略缺少有效的证书指纹")
        value = normalized_fingerprint(raw)
        if len(value) != length:
            raise RuntimeError("签名策略缺少有效的证书指纹")
        policy[key] = value
    if policy["certificate_kind"] == "apple-development" and not policy.get("team_id"):
        raise RuntimeError("Apple Development 策略缺少 Team ID")
    if not isinstance(policy.get("designated_requirement"), str) or not policy["designated_requirement"].strip():
        raise RuntimeError("签名策略缺少 designated requirement")
    return policy


def check_bundle(app):
    app = Path(app)
    if app.is_symlink() or not app.is_dir():
        raise RuntimeError("应用包必须是实际目录，不能是符号链接")
    try:
        with (app / "Contents" / "Info.plist").open("rb") as file:
            info = plistlib.load(file)
    except (OSError, ValueError, plistlib.InvalidFileException) as error:
        raise RuntimeError("无效的 macOS 应用包") from error
    if info.get("CFBundleIdentifier") != BUNDLE_ID or info.get("CFBundleName") != APP_NAME:
        raise RuntimeError("应用包不是 ScreenHop Dev")
    executable = info.get("CFBundleExecutable")
    if not isinstance(executable, str) or not executable or Path(executable).name != executable:
        raise RuntimeError("应用包可执行文件名无效")
    binary = app / "Contents" / "MacOS" / executable
    if not binary.is_file() or binary.is_symlink():
        raise RuntimeError("应用包缺少可执行文件")
    build_info_path = app / "Contents" / "Resources" / "build-info.json"
    build_info = None
    if build_info_path.is_symlink():
        raise RuntimeError("Dev 构建元数据不能是符号链接")
    if build_info_path.exists():
        try:
            with build_info_path.open(encoding="utf-8") as file:
                build_info = json.load(file)
        except (OSError, ValueError) as error:
            raise RuntimeError("Dev 构建元数据无效") from error
        if not isinstance(build_info, dict):
            raise RuntimeError("Dev 构建元数据必须为对象")
        expected = {
            "schema_version": 1,
            "dev_build": True,
            "app_name": APP_NAME,
            "bundle_id": BUNDLE_ID,
            "features": ["macos-dev"],
            "version": info.get("CFBundleShortVersionString"),
            "build_version": info.get("CFBundleVersion"),
        }
        for key, value in expected.items():
            if build_info.get(key) != value:
                raise RuntimeError("Dev 构建元数据与 Info.plist 不一致")
        if not isinstance(build_info.get("target"), str):
            raise RuntimeError("Dev 构建元数据缺少 target")
    return {"info": info, "binary": binary, "build_info": build_info}


def architectures(app):
    bundle = check_bundle(app)
    result = output(["lipo", "-archs", bundle["binary"]]).split()
    mapped = [{"aarch64": "arm64", "arm64": "arm64", "x86_64": "x86_64"}.get(item, item) for item in result]
    if not mapped or len(mapped) != len(set(mapped)) or any(item not in ("arm64", "x86_64") for item in mapped):
        raise RuntimeError("应用包含不支持或重复的 Mach-O 架构")
    return sorted(mapped)


def requirement(app, arch=None):
    args = ["codesign", "-d", "-r-"]
    if arch:
        args.extend(["--arch", arch])
    args.append(app)
    result = run(args, capture_output=True, text=True)
    for line in (result.stdout + result.stderr).splitlines():
        line = line.strip().removeprefix("#").lstrip()
        if line.startswith("designated => "):
            return line.removeprefix("designated => ").strip()
    raise RuntimeError("无法读取 designated requirement")


def is_adhoc(app, arch=None):
    args = ["codesign", "-dv"]
    if arch:
        args.extend(["--arch", arch])
    args.append(app)
    result = run(args, capture_output=True, text=True)
    return "Signature=adhoc" in result.stdout + result.stderr


def verify_integrity(app, requirement_text=None, arch=None):
    args = ["codesign", "--verify", "--deep", "--strict", "--verbose=2"]
    if arch:
        args.extend(["--arch", arch])
    else:
        args.append("--all-architectures")
    if requirement_text:
        args.extend(["-R", requirement_text])
    args.append(app)
    run(args, stdout=subprocess.DEVNULL)


def _architecture_report(app, arch):
    if is_adhoc(app, arch):
        raise RuntimeError("拒绝 ad-hoc 签名的 Dev 应用")
    req = requirement(app, arch)
    verify_integrity(app, req, arch)
    details = run(["codesign", "-dv", "--verbose=4", "--arch", arch, app], capture_output=True, text=True)
    diagnostic = details.stdout + details.stderr
    identifier = next((line.partition("=")[2].strip() for line in diagnostic.splitlines() if line.startswith("Identifier=")), None)
    team = next((line.partition("=")[2].strip() for line in diagnostic.splitlines() if line.startswith("TeamIdentifier=")), None)
    cdhash = next((line.partition("=")[2].strip().lower() for line in diagnostic.splitlines() if line.startswith("CDHash=")), None)
    if identifier != BUNDLE_ID or not cdhash:
        raise RuntimeError("签名 Identifier 或 CDHash 无效")
    with tempfile.TemporaryDirectory(prefix="screenhop-certificate-") as temporary:
        prefix = Path(temporary) / "certificate-"
        run(["codesign", "-d", "--arch", arch, "--extract-certificates", prefix, app], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        leaf = Path(str(prefix) + "0")
        if not leaf.is_file():
            raise RuntimeError("签名中缺少 DER 叶证书")
        der = leaf.read_bytes()
    return {
        "certificate_sha1": hashlib.sha1(der).hexdigest(),
        "certificate_sha256": hashlib.sha256(der).hexdigest(),
        "team_id": team if team and team != "not set" else None,
        "designated_requirement": req,
        "cdhash": cdhash,
    }


def verify_app(app, policy, expected_arches=None, require_host=False):
    app = Path(app)
    bundle = check_bundle(app)
    found_arches = architectures(app)
    if expected_arches is not None and found_arches != sorted(expected_arches):
        raise RuntimeError("应用 Mach-O 架构与预期不符")
    if require_host:
        host = {"arm64": "arm64", "aarch64": "arm64", "x86_64": "x86_64"}.get(platform.machine())
        if host not in found_arches:
            raise RuntimeError("应用不支持当前 Mac 架构")
    build_info = bundle["build_info"]
    if not build_info:
        raise RuntimeError("Dev 应用缺少签名覆盖的构建元数据")
    target_arches = {
        "aarch64-apple-darwin": ["arm64"],
        "x86_64-apple-darwin": ["x86_64"],
        "universal": ["arm64", "x86_64"],
    }
    if target_arches.get(build_info["target"]) != found_arches:
        raise RuntimeError("Dev 构建 target 与实际 Mach-O 架构不一致")
    build_version = bundle["info"].get("CFBundleVersion")
    if not isinstance(build_version, str) or not re.fullmatch(r"[0-9]{1,4}(?:\.[0-9]{1,2}){0,2}", build_version):
        raise RuntimeError("CFBundleVersion 必须是合法的数字版本")
    reports = {arch: _architecture_report(app, arch) for arch in found_arches}
    wanted_req = policy["designated_requirement"]
    verify_integrity(app, wanted_req)
    for arch, report in reports.items():
        if report["certificate_sha1"] != policy["certificate_sha1"] or report["certificate_sha256"] != policy["certificate_sha256"]:
            raise RuntimeError("签名叶证书与固定策略不匹配")
        if policy["certificate_kind"] == "apple-development" and report["team_id"] != policy["team_id"]:
            raise RuntimeError("签名 Team ID 与固定策略不匹配")
        verify_integrity(app, wanted_req, arch)
        if report["designated_requirement"] != wanted_req:
            raise RuntimeError("签名 designated requirement 与固定基线有变化")
    return {
        "bundle_id": BUNDLE_ID,
        "app_name": APP_NAME,
        "version": bundle["info"].get("CFBundleShortVersionString"),
        "build_version": bundle["info"].get("CFBundleVersion"),
        "architectures": found_arches,
        "per_arch": reports,
        "build_info": build_info,
    }


def verify_compatible(old, new):
    old_arches = architectures(old)
    new_arches = architectures(new)
    common = sorted(set(old_arches) & set(new_arches))
    if not common:
        raise RuntimeError("新旧 Dev 应用没有兼容的 Mach-O 架构")
    old_requirements = {requirement(old, arch) for arch in old_arches}
    new_requirements = {requirement(new, arch) for arch in new_arches}
    for value in sorted(old_requirements):
        verify_integrity(new, value)
    for value in sorted(new_requirements):
        verify_integrity(old, value)


def main():
    parser = argparse.ArgumentParser(description="ScreenHop Dev 固定身份验签与可信本地基线提取")
    commands = parser.add_subparsers(dest="command", required=True)
    verify = commands.add_parser("verify")
    verify.add_argument("app", type=Path)
    verify.add_argument("--policy", type=Path, default=DEFAULT_POLICY)
    verify.add_argument("--require-host", action="store_true")
    candidate = commands.add_parser("candidate")
    candidate.add_argument("app", type=Path)
    candidate.add_argument("--certificate-kind", choices=("apple-development", "self-signed"), required=True)
    candidate.add_argument("--expected-certificate-sha256", required=True)
    args = parser.parse_args()
    try:
        if args.command == "verify":
            result = verify_app(args.app, load_policy(args.policy), require_host=args.require_host)
        else:
            result = candidate_policy(args.app, args.certificate_kind)
            expected = normalized_fingerprint(args.expected_certificate_sha256)
            if len(expected) != 64 or result["certificate_sha256"] != expected:
                raise RuntimeError("本地应用证书与独立提供的可信 SHA-256 不符")
        print(json.dumps(result, ensure_ascii=False, indent=2))
        return 0
    except (OSError, RuntimeError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"验签失败：{error}\n")


def default_build_version():
    from datetime import datetime, timezone
    now = datetime.now(timezone.utc)
    return f"{now.year}.{now.month}.{now.day}"


def candidate_policy(app, certificate_kind):
    if certificate_kind not in ("apple-development", "self-signed"):
        raise RuntimeError("证书类型必须为 apple-development 或 self-signed")
    check_bundle(app)
    found_arches = architectures(app)
    reports = {arch: _architecture_report(app, arch) for arch in found_arches}
    if len({item["certificate_sha1"] for item in reports.values()}) != 1 or len({item["certificate_sha256"] for item in reports.values()}) != 1 or len({item["designated_requirement"] for item in reports.values()}) != 1:
        raise RuntimeError("各架构签名身份或 designated requirement 不一致")
    leaf = reports[found_arches[0]]
    return {
        "schema_version": 1,
        "configured": False,
        "bundle_id": BUNDLE_ID,
        "app_name": APP_NAME,
        "certificate_kind": certificate_kind,
        "certificate_sha1": leaf["certificate_sha1"],
        "certificate_sha256": leaf["certificate_sha256"],
        "team_id": leaf["team_id"] if certificate_kind == "apple-development" else None,
        "designated_requirement": leaf["designated_requirement"],
    }


if __name__ == "__main__":
    raise SystemExit(main())
