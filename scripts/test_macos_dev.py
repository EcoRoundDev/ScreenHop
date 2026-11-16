import importlib.util
import io
import json
from pathlib import Path
import plistlib
import subprocess
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from unittest.mock import call, patch


spec = importlib.util.spec_from_file_location("macos_dev", Path(__file__).with_name("macos-dev.py"))
dev = importlib.util.module_from_spec(spec)
spec.loader.exec_module(dev)


class InstallTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="screenhop-dev-test-")
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.app = self.home / "staging" / "ScreenHop Dev.app"
        self.destination = self.home / "Applications" / "ScreenHop Dev.app"
        for app, value in [(self.app, "new"), (self.destination, "old")]:
            contents = app / "Contents"
            (contents / "MacOS").mkdir(parents=True)
            (contents / "Resources").mkdir()
            (contents / "MacOS" / "screenhop").write_bytes(value.encode())
            with (contents / "Info.plist").open("wb") as file:
                plistlib.dump({"CFBundleIdentifier": dev.signing.BUNDLE_ID, "CFBundleName": dev.signing.APP_NAME, "CFBundleExecutable": "screenhop", "CFBundleShortVersionString": "1.2.3", "CFBundleVersion": "2026.11.16"}, file)
            (contents / "Resources" / "build-info.json").write_text(json.dumps({"schema_version": 1, "dev_build": True, "app_name": dev.signing.APP_NAME, "bundle_id": dev.signing.BUNDLE_ID, "features": ["macos-dev"], "version": "1.2.3", "build_version": "2026.11.16", "target": "aarch64-apple-darwin"}))
        self.home_patch = patch.object(dev.Path, "home", return_value=self.home)
        self.home_patch.start()
        self.addCleanup(self.home_patch.stop)
        for name in ["verify", "same_identity", "wait_exit", "run"]:
            p = patch.object(dev, name)
            setattr(self, name, p.start())
            self.addCleanup(p.stop)
        self.pids_patch = patch.object(dev, "installed_pids", return_value=[111])
        self.pids = self.pids_patch.start()
        self.addCleanup(self.pids_patch.stop)
        self.process_patch = patch.object(dev.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, "111", ""))
        self.process = self.process_patch.start()
        self.addCleanup(self.process_patch.stop)
        self.signing_patches = []
        for name, value in [("verify_compatible", None), ("verify_app", {"ok": True})]:
            p = patch.object(dev.signing, name, return_value=value)
            setattr(self, name, p.start())
            self.signing_patches.append(p)
            self.addCleanup(p.stop)
        p = patch.object(dev.signing, "architectures", return_value=["arm64"])
        self.architectures = p.start()
        self.addCleanup(p.stop)

    def assert_old_preserved(self):
        self.assertEqual((self.destination / "Contents" / "MacOS" / "screenhop").read_text(), "old")

    def test_identity_change_does_not_stop_or_replace(self):
        self.same_identity.side_effect = subprocess.CalledProcessError(1, "codesign")
        with self.assertRaises(subprocess.CalledProcessError):
            dev.install(self.app, False, False)
        self.assert_old_preserved()
        self.wait_exit.assert_not_called()

    def test_staging_signature_failure_does_not_stop_or_replace(self):
        self.verify.side_effect = subprocess.CalledProcessError(1, "codesign")
        with self.assertRaises(subprocess.CalledProcessError):
            dev.install(self.app, False, False)
        self.assert_old_preserved()
        self.wait_exit.assert_not_called()

    def test_production_instance_is_never_stopped(self):
        self.process.return_value = subprocess.CompletedProcess([], 0, "222", "")
        with self.assertRaisesRegex(RuntimeError, "其他 ScreenHop"):
            dev.install(self.app, False, False)
        self.assert_old_preserved()
        self.wait_exit.assert_not_called()

    def test_failed_replace_restores_old(self):
        original = Path.rename
        def rename(path, target):
            if path.name == "ScreenHop Dev.app" and path.parent.name.startswith(".screenhop-dev-install-"):
                raise OSError("mock replacement failure")
            return original(path, target)
        with patch.object(Path, "rename", new=rename):
            with self.assertRaisesRegex(OSError, "mock replacement failure"):
                dev.install(self.app, False, False)
        self.assert_old_preserved()

    def test_failed_launch_restores_old(self):
        self.run.side_effect = subprocess.CalledProcessError(1, "open")
        with self.assertRaises(subprocess.CalledProcessError):
            dev.install(self.app, False, False)
        self.assert_old_preserved()

    def test_success_keeps_old_backup_and_can_skip_open(self):
        dev.install(self.app, False, True)
        self.assertEqual((self.destination / "Contents" / "MacOS" / "screenhop").read_text(), "new")
        backups = list((self.home / "Applications").glob(".screenhop-dev-install-*/ScreenHop Dev.previous.app/Contents/MacOS/screenhop"))
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_text(), "old")
        self.run.assert_not_called()

    def test_policy_copy_is_verified_before_process_stop_and_after_install(self):
        policy = {"configured": True}
        dev.install(self.app, False, True, policy=policy)
        self.assertEqual(self.verify_app.call_count, 3)
        self.verify_app.assert_has_calls([
            call(self.app, policy, require_host=True),
            call(unittest.mock.ANY, policy, require_host=True),
            call(self.destination, policy, require_host=True),
        ])
        self.assertEqual(self.verify_compatible.call_count, 3)
        self.wait_exit.assert_called_once_with([111])

    def test_source_policy_failure_does_not_copy_stop_or_replace(self):
        self.verify_app.side_effect = RuntimeError("source verification failed")
        with patch.object(dev.shutil, "copytree") as copy, self.assertRaisesRegex(RuntimeError, "source verification failed"):
            dev.install(self.app, False, True, policy={"configured": True})
        copy.assert_not_called()
        self.pids.assert_not_called()
        self.wait_exit.assert_not_called()
        self.assert_old_preserved()

    def test_prepared_policy_failure_does_not_stop_or_replace(self):
        self.verify_app.side_effect = [None, RuntimeError("prepared verification failed")]
        with self.assertRaisesRegex(RuntimeError, "prepared verification failed"):
            dev.install(self.app, False, True, policy={"configured": True})
        self.pids.assert_not_called()
        self.wait_exit.assert_not_called()
        self.assert_old_preserved()

    def test_prepared_compatibility_failure_does_not_stop_or_replace(self):
        self.verify_compatible.side_effect = [None, RuntimeError("prepared incompatible")]
        with self.assertRaisesRegex(RuntimeError, "prepared incompatible"):
            dev.install(self.app, False, True, policy={"configured": True})
        self.pids.assert_not_called()
        self.wait_exit.assert_not_called()
        self.assert_old_preserved()

    def test_installed_policy_failure_restores_old(self):
        self.verify_app.side_effect = [None, None, RuntimeError("installed verification failed")]
        with self.assertRaisesRegex(RuntimeError, "installed verification failed"):
            dev.install(self.app, False, True, policy={"configured": True})
        self.wait_exit.assert_called_once_with([111])
        self.assert_old_preserved()
        self.run.assert_not_called()

    def test_installed_compatibility_failure_restores_old(self):
        self.verify_compatible.side_effect = [None, None, RuntimeError("installed incompatible")]
        with self.assertRaisesRegex(RuntimeError, "installed incompatible"):
            dev.install(self.app, False, True, policy={"configured": True})
        self.assert_old_preserved()
        self.run.assert_not_called()


class SigningTests(unittest.TestCase):
    def test_requirement_delegates_arch_to_shared_module(self):
        with patch.object(dev.signing, "requirement", return_value="designated req") as requirement:
            self.assertEqual(dev.requirement("app", "arm64"), "designated req")
        requirement.assert_called_once_with("app", "arm64")

    def test_same_identity_uses_shared_all_architecture_compatibility(self):
        with patch.object(dev, "verify"), patch.object(dev.signing, "verify_compatible") as compatible:
            dev.same_identity("old", "new", False)
        compatible.assert_called_once_with("old", "new")

    def test_adhoc_transition_checks_every_architecture(self):
        with patch.object(dev, "verify"), patch.object(dev.signing, "architectures", side_effect=[["arm64", "x86_64"], ["arm64"]]), patch.object(dev, "adhoc", side_effect=[True, False]), redirect_stdout(io.StringIO()):
            with self.assertRaisesRegex(RuntimeError, "降级"):
                dev.same_identity("old", "new", True)

    def test_build_version_is_numeric_and_bounded(self):
        for version in ["0", "1", "1.2", "1.01", "1234.99.0"]:
            self.assertEqual(dev.validate_build_version(version), version)
        for version in ["12345", "1.2.3.4", "1.123", "v1.2"]:
            with self.subTest(version=version), self.assertRaises(ValueError):
                dev.validate_build_version(version)

    def test_output_must_be_new_screenhop_dev_app(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaises(ValueError):
                dev.validate_output(Path(temp) / "other.app")
            app = Path(temp) / "ScreenHop Dev.app"
            app.mkdir()
            with self.assertRaises(FileExistsError):
                dev.validate_output(app)

    def test_sign_app_uses_explicit_keychain(self):
        with patch.object(dev, "run") as run:
            dev.sign_app(Path("ScreenHop Dev.app"), "CERT-SHA1", Path("temporary.keychain-db"))
        run.assert_called_once_with(["codesign", "--force", "--sign", "CERT-SHA1", "--keychain", Path("temporary.keychain-db"), Path("ScreenHop Dev.app")])

    def test_binary_cli_requires_ci_inputs_and_rejects_adhoc(self):
        with patch.object(dev.sys, "platform", "darwin"), patch.object(dev.sys, "argv", ["macos-dev.py", "--identity", "stable", "--binary", "screenhop"]), redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                dev.main()
        with patch.object(dev.sys, "platform", "darwin"), patch.object(dev.sys, "argv", ["macos-dev.py", "--adhoc"]), patch.dict(dev.os.environ, {"GITHUB_ACTIONS": "true"}), redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit):
                dev.main()

    def test_install_from_loads_policy_without_build_or_sign(self):
        with patch.object(dev.sys, "platform", "darwin"), patch.object(dev.sys, "argv", ["macos-dev.py", "--install-from", "ScreenHop Dev.app", "--no-open"]), patch.object(dev.signing, "load_policy", return_value={"configured": True}) as policy, patch.object(dev, "install") as install, patch.object(dev, "output") as output, patch.object(dev, "run") as run, patch.object(dev, "sign_app") as sign:
            dev.main()
        policy.assert_called_once_with()
        install.assert_called_once_with(Path("ScreenHop Dev.app"), False, True, policy={"configured": True})
        output.assert_not_called()
        run.assert_not_called()
        sign.assert_not_called()

    def test_install_from_rejects_external_policy_before_any_operation(self):
        with patch.object(dev.sys, "platform", "darwin"), patch.object(dev.sys, "argv", ["macos-dev.py", "--install-from", "ScreenHop Dev.app", "--policy", "downloaded-policy.json"]), patch.object(dev.signing, "load_policy") as policy, patch.object(dev, "install") as install, patch.object(dev, "run") as run, patch.object(dev, "output") as output, redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as raised:
                dev.main()
        self.assertEqual(raised.exception.code, 2)
        policy.assert_not_called()
        install.assert_not_called()
        run.assert_not_called()
        output.assert_not_called()

    def test_invalid_cli_modes_fail_before_build_or_sign(self):
        for arguments in [["--identity=-"], ["--install-from", "app", "--identity", "stable"], ["--install-from", "app", "--keychain", "private"], ["--identity", "stable", "--target", "universal"], ["--identity", "stable", "--binary", "binary", "--build-only", "--target", "universal", "--output", "ScreenHop Dev.app", "--policy", "policy"]]:
            with self.subTest(arguments=arguments), patch.object(dev.sys, "platform", "darwin"), patch.object(dev.sys, "argv", ["macos-dev.py"] + arguments), patch.object(dev, "run") as run, patch.object(dev, "output") as output, redirect_stderr(io.StringIO()):
                with self.assertRaises(SystemExit):
                    dev.main()
                run.assert_not_called()
                output.assert_not_called()

    def test_assemble_app_writes_metadata_before_signing_and_checks_arches(self):
        with tempfile.TemporaryDirectory(prefix="screenhop-assemble-test-") as temp:
            root = Path(temp)
            binary = root / "screenhop"
            binary.write_bytes(b"binary")
            app = root / "ScreenHop Dev.app"
            with patch.object(dev, "output", return_value="arm64"):
                result = dev.assemble_app(binary, app, "2026.11.16", {"version": "1.2.3", "target": "aarch64-apple-darwin", "commit": "abc", "run_id": "10", "run_attempt": "2"})
            self.assertEqual(result, app)
            with (app / "Contents" / "Info.plist").open("rb") as file:
                info = plistlib.load(file)
            build_info = json.loads((app / "Contents" / "Resources" / "build-info.json").read_text())
            self.assertEqual(info["CFBundleVersion"], "2026.11.16")
            self.assertEqual(build_info["build_version"], info["CFBundleVersion"])
            self.assertTrue(build_info["dev_build"])
            self.assertEqual(build_info["features"], ["macos-dev"])
            self.assertEqual(build_info["commit"], "abc")
            with self.assertRaises(FileExistsError):
                dev.assemble_app(binary, app, "2026.11.16", {"version": "1.2.3", "target": "aarch64-apple-darwin"})

    def test_binary_build_mode_uses_build_info_and_skips_cargo_compile(self):
        with tempfile.TemporaryDirectory(prefix="screenhop-binary-test-") as temp:
            root = Path(temp)
            binary = root / "screenhop"
            binary.write_bytes(b"binary")
            build_info = root / "build-info.json"
            build_info.write_text(json.dumps({"version": "1.2.3", "commit": "abc", "run_id": "10", "run_attempt": "2"}))
            output = json.dumps({"target_directory": temp, "packages": [{"name": "screenhop", "version": "1.2.3"}]})
            with patch.object(dev.sys, "platform", "darwin"), patch.object(dev.sys, "argv", ["macos-dev.py", "--identity", "CERT-SHA1", "--binary", str(binary), "--build-only", "--target", "aarch64-apple-darwin", "--output", str(root / "ScreenHop Dev.app"), "--policy", "policy.json", "--keychain", str(root / "ci.keychain"), "--build-info", str(build_info)]), patch.object(dev.signing, "load_policy", return_value={"configured": True}) as load_policy, patch.object(dev, "output", return_value=output) as output_command, patch.object(dev, "assemble_app", return_value=root / "ScreenHop Dev.app") as assemble, patch.object(dev, "sign_app") as sign, patch.object(dev.signing, "verify_app", return_value={"ok": True}) as verify_app, redirect_stdout(io.StringIO()):
                dev.main()
            load_policy.assert_called_once_with(Path("policy.json"))
            output_command.assert_not_called()
            self.assertEqual(assemble.call_args.args[0], binary)
            self.assertEqual(assemble.call_args.args[3]["commit"], "abc")
            self.assertEqual(assemble.call_args.args[3]["run_id"], "10")
            self.assertEqual(assemble.call_args.args[3]["run_attempt"], "2")
            self.assertEqual(assemble.call_args.args[3]["version"], "1.2.3")
            self.assertEqual(assemble.call_args.args[3]["target"], "aarch64-apple-darwin")
            sign.assert_called_once_with(root / "ScreenHop Dev.app", "CERT-SHA1", Path(str(root / "ci.keychain")))
            verify_app.assert_called_once_with(root / "ScreenHop Dev.app", {"configured": True}, expected_arches=["arm64"])

    def test_binary_metadata_target_must_match_cli(self):
        with tempfile.TemporaryDirectory(prefix="screenhop-binary-target-test-") as temp:
            root = Path(temp)
            binary = root / "screenhop"
            binary.write_bytes(b"binary")
            build_info = root / "build-info.json"
            build_info.write_text(json.dumps({"version": "1.2.3", "target": "x86_64-apple-darwin"}))
            with patch.object(dev.sys, "platform", "darwin"), patch.object(dev.sys, "argv", ["macos-dev.py", "--identity", "CERT-SHA1", "--binary", str(binary), "--build-only", "--target", "aarch64-apple-darwin", "--output", str(root / "ScreenHop Dev.app"), "--policy", "policy.json", "--keychain", str(root / "ci.keychain"), "--build-info", str(build_info)]), patch.object(dev.signing, "load_policy", return_value={"configured": True}), patch.object(dev, "output") as output, patch.object(dev, "assemble_app") as assemble, redirect_stderr(io.StringIO()):
                with self.assertRaisesRegex(ValueError, "target 与 --target"):
                    dev.main()
            output.assert_not_called()
            assemble.assert_not_called()

    def test_universal_binary_requires_exact_two_architectures(self):
        with tempfile.TemporaryDirectory(prefix="screenhop-universal-test-") as temp:
            root = Path(temp)
            binary = root / "screenhop"
            binary.write_bytes(b"binary")
            app = root / "ScreenHop Dev.app"
            with patch.object(dev, "output", return_value="arm64 x86_64") as output:
                dev.assemble_app(binary, app, "2026.11.16", {"version": "1.2.3", "target": "universal"})
            self.assertEqual(output.call_count, 2)
            with self.assertRaises(FileExistsError):
                dev.assemble_app(binary, app, "2026.11.16", {"version": "1.2.3", "target": "universal"})

    def test_assemble_app_rejects_wrong_architecture(self):
        with tempfile.TemporaryDirectory(prefix="screenhop-assemble-test-") as temp:
            root = Path(temp)
            binary = root / "screenhop"
            binary.write_bytes(b"binary")
            with patch.object(dev, "output", return_value="arm64 x86_64"):
                with self.assertRaises(RuntimeError):
                    dev.assemble_app(binary, root / "ScreenHop Dev.app", "2026.11.16", {"version": "1.2.3", "target": "aarch64-apple-darwin"})

    def test_build_only_never_installs_or_stops_processes(self):
        self.run_build_only(False)

    def test_signing_failure_never_installs(self):
        self.run_build_only(True)

    def test_non_host_target_is_allowed_for_build_only(self):
        self.run_build_only(False, target="x86_64-apple-darwin")

    def test_legacy_adhoc_build_only_remains_supported(self):
        self.run_build_only(False, adhoc=True)

    def run_build_only(self, fail_sign, adhoc=False, target=None):
        with tempfile.TemporaryDirectory(prefix="screenhop-dev-build-test-") as temp:
            stage = Path(temp) / "stage"
            stage.mkdir()
            meta = json.dumps({"target_directory": temp, "packages": [{"name": "screenhop", "version": "1.2.3"}]})
            commands = []
            def run(args, **kwargs):
                commands.append(args)
                if fail_sign and args[0:2] == ["codesign", "--force"]:
                    raise subprocess.CalledProcessError(1, "codesign")
                return subprocess.CompletedProcess(args, 0, "", "")
            with patch.object(dev.sys, "argv", (["macos-dev.py", "--adhoc", "--build-only"] if adhoc else ["macos-dev.py", "--identity", "stable", "--build-only"]) + (["--target", target] if target else [])), patch.object(dev.sys, "platform", "darwin"), patch.object(dev, "output", side_effect=[meta, "host: aarch64-apple-darwin"]), patch.object(dev, "run", side_effect=run), patch.object(dev, "assemble_app", return_value=stage / "ScreenHop Dev.app") as assemble, patch.object(dev, "verify"), patch.object(dev.signing, "architectures", return_value=["arm64"]), patch.object(dev.signing, "requirement", return_value="req"), patch.object(dev, "install") as install, patch.object(dev, "wait_exit") as stop, patch.object(dev.Path, "home") as home, redirect_stdout(io.StringIO()):
                if fail_sign:
                    with self.assertRaises(subprocess.CalledProcessError):
                        dev.main()
                else:
                    dev.main()
                assemble.assert_called_once()
                if target:
                    self.assertTrue(any(args[0:5] == ["cargo", "build", "--release", "--target", target] for args in commands))
                install.assert_not_called()
                stop.assert_not_called()
                home.assert_not_called()


if __name__ == "__main__":
    unittest.main()
