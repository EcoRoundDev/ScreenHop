#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time

import macos_signing as signing


TARGET_ARCHES = {
    "aarch64-apple-darwin": ["arm64"],
    "x86_64-apple-darwin": ["x86_64"],
    "universal": ["arm64", "x86_64"],
}


def run(args, **kwargs):
    return subprocess.run([str(arg) for arg in args], check=True, **kwargs)


def output(args):
    return run(args, capture_output=True, text=True).stdout.strip()


def requirement(app, arch=None):
    return signing.requirement(app, arch)


def adhoc(app, arch=None):
    return signing.is_adhoc(app, arch)


def verify(app):
    signing.verify_integrity(app)


def same_identity(old, new, allow_adhoc):
    verify(old)
    verify(new)
    if allow_adhoc:
        for app in (old, new):
            for arch in signing.architectures(app):
                if not adhoc(app, arch):
                    raise RuntimeError("拒绝将稳定签名的已安装开发包降级为 ad-hoc；请保留原签名身份")
        print("显式 ad-hoc 模式：允许构建身份变化，可能需要重新授权")
    else:
        signing.verify_compatible(old, new)


def installed_pids(binary):
    result = subprocess.run(["lsof", "-t", str(binary)], capture_output=True, text=True)
    if result.returncode not in (0, 1):
        raise RuntimeError("无法核查旧开发进程，请先手动退出后重试")
    pids = []
    for item in result.stdout.split():
        pid = int(item)
        command = subprocess.run(["ps", "-p", str(pid), "-o", "comm="], capture_output=True, text=True)
        if command.returncode == 0 and command.stdout.strip() == str(binary):
            pids.append(pid)
    return pids


def wait_exit(pids):
    for pid in pids:
        os.kill(pid, signal.SIGTERM)
    deadline = time.monotonic() + 10
    for pid in pids:
        while True:
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                break
            if time.monotonic() > deadline:
                raise RuntimeError("旧开发实例没有退出；未替换应用，请手动退出后重试")
            time.sleep(0.1)


def validate_build_version(version):
    if not isinstance(version, str) or not re.fullmatch(r"[0-9]{1,4}(?:\.[0-9]{1,2}){0,2}", version):
        raise ValueError("build-version 必须是合法数字版本：首段最多四位，最多两个两位数字后缀")
    return version


def validate_output(app):
    if app.name != signing.APP_NAME + ".app":
        raise ValueError("output 必须指向 ScreenHop Dev.app")
    if app.exists() or app.is_symlink():
        raise FileExistsError("拒绝覆盖已存在的 output: " + str(app))


def binary_architectures(binary, target):
    if target not in TARGET_ARCHES:
        raise ValueError("不支持的 macOS target: " + str(target))
    arches = output(["lipo", "-archs", binary]).split()
    if len(arches) != len(TARGET_ARCHES[target]) or set(arches) != set(TARGET_ARCHES[target]):
        raise RuntimeError("二进制架构与 target 不符: " + " ".join(arches))
    return arches


def assemble_app(binary: Path, output: Path, build_version: str, build_info: dict) -> Path:
    binary, app = Path(binary), Path(output)
    validate_output(app)
    validate_build_version(build_version)
    if not binary.is_file():
        raise ValueError("预构建二进制不存在: " + str(binary))
    if not isinstance(build_info, dict):
        raise ValueError("build-info 必须是 JSON 对象")
    version, target = build_info.get("version"), build_info.get("target")
    if not isinstance(version, str) or not version:
        raise ValueError("build-info 缺少 version")
    binary_architectures(binary, target)
    root = Path(__file__).resolve().parent.parent
    contents = app / "Contents"
    (contents / "MacOS").mkdir(parents=True)
    (contents / "Resources").mkdir()
    executable = contents / "MacOS" / "screenhop"
    shutil.copy2(binary, executable)
    executable.chmod(executable.stat().st_mode | 0o111)
    shutil.copy2(root / "assets" / "ScreenHop.icns", contents / "Resources" / "ScreenHop.icns")
    info = {"CFBundleIdentifier": signing.BUNDLE_ID, "CFBundleName": signing.APP_NAME, "CFBundleDisplayName": signing.APP_NAME, "CFBundleExecutable": "screenhop", "CFBundlePackageType": "APPL", "CFBundleInfoDictionaryVersion": "6.0", "CFBundleShortVersionString": version, "CFBundleVersion": build_version, "CFBundleIconFile": "ScreenHop.icns", "NSHighResolutionCapable": True, "LSUIElement": True}
    with (contents / "Info.plist").open("wb") as file:
        plistlib.dump(info, file)
    metadata = dict(build_info)
    metadata.update(schema_version=1, dev_build=True, app_name=signing.APP_NAME, bundle_id=signing.BUNDLE_ID, features=["macos-dev"], version=version, build_version=build_version, target=target)
    (contents / "Resources" / "build-info.json").write_text(json.dumps(metadata, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    binary_architectures(executable, target)
    return app


def sign_app(app: Path, identity: str, keychain: Path | None = None) -> None:
    command = ["codesign", "--force", "--sign", identity]
    if keychain is not None:
        command.extend(["--keychain", keychain])
    command.append(app)
    run(command)


def main():
    parser = argparse.ArgumentParser(description="构建、签名并安全安装 ScreenHop Dev；不修改钥匙串或 TCC")
    signer = parser.add_mutually_exclusive_group(required=True)
    signer.add_argument("--identity", help="长期保留的代码签名身份，推荐证书 SHA-1 指纹")
    signer.add_argument("--adhoc", action="store_true", help="显式临时签名，不能保证保留授权")
    signer.add_argument("--install-from", type=Path, help="验证并安装已签名 app，不编译或重新签名")
    parser.add_argument("--build-only", action="store_true", help="只构建签名 app，不安装、不终止进程、不启动")
    parser.add_argument("--no-open", action="store_true", help="安装后不启动")
    parser.add_argument("--target", choices=TARGET_ARCHES, help="默认本机 target；非本机仅支持 build-only")
    parser.add_argument("--binary", type=Path, help="组包已用 macos-dev feature 构建的二进制")
    parser.add_argument("--output", type=Path, help="尚不存在的 ScreenHop Dev.app 完整路径")
    parser.add_argument("--keychain", type=Path, help="显式用于 codesign 的钥匙串")
    parser.add_argument("--policy", type=Path, help="受信的公开签名策略")
    parser.add_argument("--build-version", help="CFBundleVersion 数字版本")
    parser.add_argument("--build-info", type=Path, help="签名前写入的公开构建信息 JSON 文件")
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("仅支持 macOS")
    if args.identity == "-":
        parser.error("临时签名请显式使用 --adhoc")
    ci = os.environ.get("GITHUB_ACTIONS", "").lower() == "true"
    if ci and args.adhoc:
        parser.error("GitHub Actions 禁止 --adhoc")
    if args.install_from is not None:
        if args.policy is not None:
            parser.error("--install-from 只能使用受信仓库的固定签名策略")
        if any((args.build_only, args.target, args.binary, args.output, args.keychain, args.build_version, args.build_info)):
            parser.error("--install-from 不能与构建或签名参数组合")
        policy = signing.load_policy()
        install(args.install_from, False, args.no_open, policy=policy)
        return
    if args.target == "universal" and args.binary is None:
        parser.error("--target universal 仅支持 --binary 模式")
    if args.binary is not None:
        if not all((args.build_only, args.target, args.output, args.policy)):
            parser.error("--binary 必须同时指定 --build-only --target --output --policy")
        if args.keychain is None:
            parser.error("--binary 签名必须显式指定 --keychain")
    if args.output is not None:
        validate_output(args.output)
    policy = signing.load_policy(args.policy) if args.policy else None
    build_version = validate_build_version(args.build_version or signing.default_build_version())
    build_info = {}
    if args.build_info is not None:
        with args.build_info.open(encoding="utf-8") as file:
            build_info = json.load(file)
        if not isinstance(build_info, dict):
            raise ValueError("build-info 必须是 JSON 对象")
    if args.adhoc:
        print("警告：ad-hoc 构建的权限可能在每次重编后失效")
    root = Path(__file__).resolve().parent.parent
    metadata = None
    if args.binary is None or "version" not in build_info:
        metadata = json.loads(output(["cargo", "metadata", "--no-deps", "--format-version", "1", "--manifest-path", root / "Cargo.toml"]))
        package = next(p for p in metadata["packages"] if p["name"] == "screenhop")
        build_info["version"] = package["version"]
    if args.binary is not None:
        target, source = args.target, args.binary
        metadata_target = build_info.get("target")
        if metadata_target is not None and metadata_target != target:
            raise ValueError("build-info target 与 --target 不一致")
    else:
        host = next(line.removeprefix("host: ") for line in output(["rustc", "-vV"]).splitlines() if line.startswith("host: "))
        target = args.target or host
        if target not in TARGET_ARCHES:
            parser.error("不支持的本机 target: " + target)
        if target != host and not args.build_only:
            parser.error("非本机 target 必须使用 --build-only")
        run(["cargo", "build", "--release", "--target", target, "--features", "macos-dev", "--manifest-path", root / "Cargo.toml"])
        source = Path(metadata["target_directory"]) / target / "release" / "screenhop"
    app = args.output
    if app is None:
        app = Path(tempfile.mkdtemp(prefix="screenhop-dev-build-")) / (signing.APP_NAME + ".app")
    build_info["target"] = target
    assemble_app(source, app, build_version, build_info)
    sign_app(app, "-" if args.adhoc else args.identity, args.keychain)
    if policy is not None:
        report = signing.verify_app(app, policy, expected_arches=TARGET_ARCHES[target])
        print(json.dumps(report, indent=2, ensure_ascii=False))
    else:
        verify(app)
        for arch in signing.architectures(app):
            print("新 DR (" + arch + "):", requirement(app, arch))
    print("已构建:", app)
    if not args.build_only:
        install(app, args.adhoc, args.no_open, policy=policy)


def install(app, allow_adhoc, no_open, policy=None):
    app = Path(app)
    if policy is not None:
        signing.verify_app(app, policy, require_host=True)
    else:
        signing.check_bundle(app)
        verify(app)
    parent = Path.home() / "Applications"
    parent.mkdir(exist_ok=True)
    destination = parent / (signing.APP_NAME + ".app")
    if destination.is_symlink():
        raise RuntimeError("拒绝替换符号链接应用")
    if destination.exists():
        with (destination / "Contents" / "Info.plist").open("rb") as file:
            if plistlib.load(file).get("CFBundleIdentifier") != signing.BUNDLE_ID:
                raise RuntimeError("目标不是 ScreenHop Dev，拒绝替换")
        if policy is not None:
            signing.verify_compatible(destination, app)
        else:
            same_identity(destination, app, allow_adhoc)
    transaction = Path(tempfile.mkdtemp(prefix=".screenhop-dev-install-", dir=parent))
    prepared = transaction / (signing.APP_NAME + ".app")
    shutil.copytree(app, prepared, symlinks=True)
    if policy is not None:
        signing.verify_app(prepared, policy, require_host=True)
        if destination.exists():
            signing.verify_compatible(destination, prepared)
    else:
        signing.check_bundle(prepared)
        verify(prepared)
        if destination.exists():
            same_identity(destination, prepared, allow_adhoc)
    binary = destination / "Contents" / "MacOS" / "screenhop"
    pids = installed_pids(binary) if destination.exists() else []
    ports = subprocess.run(["lsof", "-nP", "-t", "-iTCP:57832", "-sTCP:LISTEN"], capture_output=True, text=True)
    if ports.returncode not in (0, 1):
        raise RuntimeError("无法核查单实例端口；未替换，请先手动检查 ScreenHop 实例")
    if any(int(pid) not in pids for pid in ports.stdout.split()):
        raise RuntimeError("其他 ScreenHop 实例占用单实例端口，请先退出生产包或其他位置的开发包")
    wait_exit(pids)
    backup = transaction / "ScreenHop Dev.previous.app"
    moved_old = False
    installed_new = False
    print("事务目录:", transaction)
    try:
        if destination.exists():
            destination.rename(backup)
            moved_old = True
        prepared.rename(destination)
        installed_new = True
        if policy is not None:
            signing.verify_app(destination, policy, require_host=True)
            if moved_old:
                signing.verify_compatible(backup, destination)
        else:
            signing.check_bundle(destination)
            verify(destination)
        if not no_open:
            run(["open", destination])
    except Exception:
        if installed_new and destination.exists():
            destination.rename(transaction / "ScreenHop Dev.failed.app")
        if moved_old:
            backup.rename(destination)
        raise
    print("已安装:", destination)
    print("事务备份目录（确认可用后可手工清理）:", transaction)
    print("首次授权请在辅助功能中选择 ScreenHop Dev；开发包沿用生产单实例限制")


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.CalledProcessError) as error:
        print("错误:", error, file=sys.stderr)
        sys.exit(1)
