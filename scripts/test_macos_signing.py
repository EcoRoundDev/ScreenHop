import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import macos_signing as signing


class SigningPolicyTests(unittest.TestCase):
    def policy(self):
        return {
            "schema_version": 1,
            "configured": True,
            "bundle_id": signing.BUNDLE_ID,
            "app_name": signing.APP_NAME,
            "certificate_kind": "apple-development",
            "certificate_sha1": "a" * 40,
            "certificate_sha256": "b" * 64,
            "team_id": "ABCDE12345",
            "designated_requirement": 'identifier "com.ecorounddev.screenhop.dev" and anchor apple generic',
        }

    def test_unconfigured_policy_fails_closed(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "policy.json"
            path.write_text(json.dumps({
                "schema_version": 1,
                "configured": False,
                "bundle_id": signing.BUNDLE_ID,
                "app_name": signing.APP_NAME,
            }), encoding="utf-8")
            with self.assertRaisesRegex(RuntimeError, "尚未配置"):
                signing.load_policy(path)

    def test_policy_requires_exact_fingerprint_characters(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "policy.json"
            for value in ("g" * 40, "aa-bb", "a" * 39):
                policy = self.policy()
                policy["certificate_sha1"] = value
                path.write_text(json.dumps(policy), encoding="utf-8")
                with self.subTest(value=value), self.assertRaisesRegex(RuntimeError, "指纹"):
                    signing.load_policy(path)

    def test_policy_normalizes_colon_separated_fingerprints(self):
        policy = self.policy()
        policy["certificate_sha1"] = ":".join(["aa"] * 20)
        policy["certificate_sha256"] = ":".join(["bb"] * 32)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "policy.json"
            path.write_text(json.dumps(policy), encoding="utf-8")
            loaded = signing.load_policy(path)
        self.assertEqual(loaded["certificate_sha1"], "aa" * 20)
        self.assertEqual(loaded["certificate_sha256"], "bb" * 32)

    def test_duplicate_architecture_alias_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory) / "ScreenHop Dev.app"
            binary = app / "Contents" / "MacOS" / "ScreenHop"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"mock")
            with (app / "Contents" / "Info.plist").open("wb") as file:
                import plistlib
                plistlib.dump({"CFBundleIdentifier": signing.BUNDLE_ID, "CFBundleName": signing.APP_NAME, "CFBundleExecutable": "ScreenHop"}, file)
            with patch.object(signing, "output", return_value="arm64 aarch64"):
                with self.assertRaisesRegex(RuntimeError, "架构"):
                    signing.architectures(app)

    def test_symlink_bundle_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            real = root / "actual.app"
            real.mkdir()
            alias = root / "alias.app"
            alias.symlink_to(real, target_is_directory=True)
            with self.assertRaisesRegex(RuntimeError, "符号链接"):
                signing.check_bundle(alias)

    def test_verify_requires_metadata_and_exact_arch_set(self):
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory) / "ScreenHop Dev.app"
            binary = app / "Contents" / "MacOS" / "ScreenHop"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"mock")
            with (app / "Contents" / "Info.plist").open("wb") as file:
                import plistlib
                plistlib.dump({"CFBundleIdentifier": signing.BUNDLE_ID, "CFBundleName": signing.APP_NAME, "CFBundleExecutable": "ScreenHop", "CFBundleShortVersionString": "1.1.5", "CFBundleVersion": "2026.11.16"}, file)
            with patch.object(signing, "output", return_value="arm64"):
                with self.assertRaisesRegex(RuntimeError, "元数据"):
                    signing.verify_app(app, self.policy(), expected_arches=["arm64"])
                with self.assertRaisesRegex(RuntimeError, "架构"):
                    signing.verify_app(app, self.policy(), expected_arches=["x86_64"])

    def test_signed_metadata_target_must_match_macho_slices(self):
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory) / "ScreenHop Dev.app"
            (app / "Contents" / "MacOS").mkdir(parents=True)
            (app / "Contents" / "Resources").mkdir()
            (app / "Contents" / "MacOS" / "ScreenHop").write_bytes(b"mock")
            info = {"CFBundleIdentifier": signing.BUNDLE_ID, "CFBundleName": signing.APP_NAME, "CFBundleExecutable": "ScreenHop", "CFBundleShortVersionString": "1.1.5", "CFBundleVersion": "2026.11.16"}
            with (app / "Contents" / "Info.plist").open("wb") as file:
                import plistlib
                plistlib.dump(info, file)
            metadata = {"schema_version": 1, "dev_build": True, "app_name": signing.APP_NAME, "bundle_id": signing.BUNDLE_ID, "features": ["macos-dev"], "version": "1.1.5", "build_version": "2026.11.16", "target": "universal"}
            (app / "Contents" / "Resources" / "build-info.json").write_text(json.dumps(metadata), encoding="utf-8")
            with patch.object(signing, "output", return_value="arm64"):
                with patch.object(signing, "_architecture_report", return_value={"certificate_sha1": "a" * 40, "certificate_sha256": "b" * 64, "team_id": "ABCDE12345", "designated_requirement": self.policy()["designated_requirement"], "cdhash": "c" * 40}):
                    with patch.object(signing, "verify_integrity"):
                        with self.assertRaisesRegex(RuntimeError, "target"):
                            signing.verify_app(app, self.policy())

    def test_verify_checks_every_architecture_against_pinned_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            app = Path(directory) / "ScreenHop Dev.app"
            (app / "Contents" / "MacOS").mkdir(parents=True)
            (app / "Contents" / "Resources").mkdir()
            (app / "Contents" / "MacOS" / "ScreenHop").write_bytes(b"mock")
            info = {"CFBundleIdentifier": signing.BUNDLE_ID, "CFBundleName": signing.APP_NAME, "CFBundleExecutable": "ScreenHop", "CFBundleShortVersionString": "1.1.5", "CFBundleVersion": "2026.11.16"}
            with (app / "Contents" / "Info.plist").open("wb") as file:
                import plistlib
                plistlib.dump(info, file)
            metadata = {"schema_version": 1, "dev_build": True, "app_name": signing.APP_NAME, "bundle_id": signing.BUNDLE_ID, "features": ["macos-dev"], "version": "1.1.5", "build_version": "2026.11.16", "target": "universal"}
            (app / "Contents" / "Resources" / "build-info.json").write_text(json.dumps(metadata), encoding="utf-8")
            report = {"certificate_sha1": "a" * 40, "certificate_sha256": "b" * 64, "team_id": "ABCDE12345", "designated_requirement": self.policy()["designated_requirement"], "cdhash": "c" * 40}
            with patch.object(signing, "output", return_value="arm64 x86_64"):
                with patch.object(signing, "_architecture_report", side_effect=[report, {**report, "certificate_sha256": "d" * 64}]):
                    with patch.object(signing, "verify_integrity") as integrity:
                        with self.assertRaisesRegex(RuntimeError, "叶证书"):
                            signing.verify_app(app, self.policy(), expected_arches=["arm64", "x86_64"])
            self.assertEqual(integrity.call_count, 2)

    def test_compatible_checks_requirements_bidirectionally_for_each_arch(self):
        requirement = {("old", "arm64"): "old-arm64", ("old", "x86_64"): "old-x86", ("new", "arm64"): "new-arm64", ("new", "x86_64"): "new-x86"}
        calls = []
        with patch.object(signing, "architectures", side_effect=[["arm64", "x86_64"], ["arm64", "x86_64"]]):
            with patch.object(signing, "requirement", side_effect=lambda app, arch: requirement[(app, arch)]):
                with patch.object(signing, "verify_integrity", side_effect=lambda app, req: calls.append((app, req))):
                    signing.verify_compatible("old", "new")
        self.assertEqual(calls, [("new", "old-arm64"), ("new", "old-x86"), ("old", "new-arm64"), ("old", "new-x86")])

    def test_no_common_architecture_rejected_before_compatibility_checks(self):
        with patch.object(signing, "architectures", side_effect=[["arm64"], ["x86_64"]]):
            with patch.object(signing, "requirement") as requirement:
                with self.assertRaisesRegex(RuntimeError, "兼容的 Mach-O"):
                    signing.verify_compatible("old", "new")
            requirement.assert_not_called()


if __name__ == "__main__":
    unittest.main()
