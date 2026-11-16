import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest import mock

SCRIPT = Path(__file__).with_name("macos-ci-signing.py")
spec = importlib.util.spec_from_file_location("macos_ci_signing", SCRIPT)
ci = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci)


class MacosCiSigningTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name).resolve()
        self.env = mock.patch.dict(os.environ, {
            "RUNNER_ENVIRONMENT": "github-hosted",
            "RUNNER_TEMP": str(self.root / "runner"),
            "GITHUB_RUN_ID": "12345",
            "GITHUB_RUN_ATTEMPT": "1",
            "GITHUB_SHA": "0123456789abcdef0123456789abcdef01234567",
            "GITHUB_WORKSPACE": str(self.root / "workspace"),
        }, clear=False)
        self.env.start()
        Path(os.environ["RUNNER_TEMP"]).mkdir()
        self.policy = {
            "schema_version": 1,
            "configured": True,
            "bundle_id": "com.ecorounddev.screenhop.dev",
            "app_name": "ScreenHop Dev",
            "certificate_kind": "self-signed",
            "certificate_sha1": "1" * 40,
            "certificate_sha256": "2" * 64,
            "team_id": None,
            "designated_requirement": 'identifier "com.ecorounddev.screenhop.dev"',
        }

    def tearDown(self):
        self.env.stop()
        self.temp.cleanup()

    def test_missing_secret_fails_without_adhoc_fallback(self):
        with mock.patch.object(ci, "_configured_policy", return_value=self.policy):
            with mock.patch.dict(os.environ, {}, clear=False):
                os.environ.pop("MACOS_DEV_CERT_P12_BASE64", None)
                os.environ.pop("MACOS_DEV_CERT_PASSWORD", None)
                with self.assertRaisesRegex(RuntimeError, "不提供 ad-hoc 回退"):
                    ci.sign()

    def test_wrong_certificate_fails_before_self_signed_trust(self):
        with mock.patch.object(ci, "_configured_policy", return_value=self.policy), \
             mock.patch.dict(os.environ, {"MACOS_DEV_CERT_P12_BASE64": "cDEy", "MACOS_DEV_CERT_PASSWORD": "secret"}), \
             mock.patch.object(ci, "_extract_leaf") as extract, \
             mock.patch.object(ci, "_run") as run:
            def write_wrong_leaf(_p12, _password, private):
                pem = private / "leaf.pem"
                der = private / "leaf.der"
                pem.write_bytes(b"pem")
                der.write_bytes(b"wrong certificate")
                return pem, der
            extract.side_effect = write_wrong_leaf
            with self.assertRaisesRegex(RuntimeError, "指纹不匹配"):
                ci.sign()
            self.assertFalse(any("add-trusted-cert" in str(call) for call in run.call_args_list))

    def test_subprocess_failure_does_not_expose_args_or_stderr(self):
        failure = ci.subprocess.CompletedProcess(["security", "import", "-P", "private-password"], 1, "", "private-password leaked")
        with mock.patch.object(ci.subprocess, "run", return_value=failure):
            with self.assertRaises(RuntimeError) as raised:
                ci._run(["security", "import", "-P", "private-password"])
        self.assertNotIn("private-password", str(raised.exception))
        self.assertNotIn("leaked", str(raised.exception))

    def test_cleanup_removes_trust_before_keychain_and_is_idempotent(self):
        private = Path(os.environ["RUNNER_TEMP"]) / "screenhop-macos-dev-signing" / "12345-1" / "private"
        private.mkdir(parents=True)
        leaf = private / "leaf.der"
        leaf.write_bytes(b"certificate")
        keychain = private / "macos-dev.keychain-db"
        keychain.write_bytes(b"keychain")
        state = {"keychain": str(keychain), "leaf_der": str(leaf), "trust_attempted": True}
        (private / "state.json").write_text(json.dumps(state))
        calls = []
        def execute(args, **_kwargs):
            calls.append([str(item) for item in args])
            if args[1] == "delete-keychain":
                Path(args[-1]).unlink()
            return ci.subprocess.CompletedProcess(args, 0, "", "")
        with mock.patch.object(ci, "_execute", side_effect=execute):
            ci.cleanup()
            ci.cleanup()
        self.assertEqual([call[1] for call in calls], ["remove-trusted-cert", "delete-keychain"])

    def test_cleanup_retries_trust_failure_and_preserves_only_retry_material(self):
        private = ci._private_dir(create=True)
        leaf = private / "leaf.der"
        keychain = private / "macos-dev.keychain-db"
        leaf.write_bytes(b"certificate")
        keychain.write_bytes(b"keychain")
        (private / "identity.p12").write_bytes(b"private")
        (private / "leaf.pem").write_bytes(b"public")
        ci._save_state(private, {"keychain": str(keychain), "leaf_der": str(leaf), "trust_attempted": True})
        calls = []
        def execute(args, **_kwargs):
            calls.append(args[1])
            if args[1] == "remove-trusted-cert" and calls.count("remove-trusted-cert") == 1:
                return ci.subprocess.CompletedProcess(args, 1, "", "operation failed")
            if args[1] == "delete-keychain":
                Path(args[-1]).unlink()
            return ci.subprocess.CompletedProcess(args, 0, "", "")
        with mock.patch.object(ci, "_execute", side_effect=execute):
            with self.assertRaisesRegex(RuntimeError, "清理失败"):
                ci.cleanup()
            self.assertEqual({path.name for path in private.iterdir()}, {"leaf.der", "state.json"})
            ci.cleanup()
            ci.cleanup()
        self.assertEqual(calls, ["remove-trusted-cert", "delete-keychain", "remove-trusted-cert"])
        self.assertFalse(private.exists())

    def test_subprocess_exception_is_redacted(self):
        failure = ci.subprocess.CalledProcessError(1, ["security", "-p", "private-password"], stderr="private-password")
        with mock.patch.object(ci.subprocess, "run", side_effect=failure):
            with self.assertRaises(RuntimeError) as raised:
                ci._run(["security", "-p", "private-password"])
        self.assertNotIn("private-password", str(raised.exception))

    def test_sign_failure_immediately_cleans_private_material(self):
        with mock.patch.object(ci, "_configured_policy", return_value=self.policy), \
             mock.patch.dict(os.environ, {"MACOS_DEV_CERT_P12_BASE64": "cDEy", "MACOS_DEV_CERT_PASSWORD": "secret"}), \
             mock.patch.object(ci, "_extract_leaf", side_effect=RuntimeError("failed extraction")):
            with self.assertRaisesRegex(RuntimeError, "failed extraction"):
                ci.sign()
        self.assertFalse(ci._private_dir().exists())

    def test_hosted_guard_prevents_local_changes(self):
        with mock.patch.dict(os.environ, {"RUNNER_ENVIRONMENT": "self-hosted"}), \
             mock.patch.object(ci, "_execute") as execute:
            with self.assertRaisesRegex(RuntimeError, "GitHub-hosted"):
                ci.cleanup()
        execute.assert_not_called()

    def test_unconfigured_policy_fails_before_secret_or_build(self):
        with mock.patch.object(ci, "_configured_policy", side_effect=RuntimeError("policy unconfigured")), \
             mock.patch.object(ci, "_run") as run:
            with self.assertRaisesRegex(RuntimeError, "policy unconfigured"):
                ci.prepare(arm64_binary="missing", x86_binary="missing")
        run.assert_not_called()

    def test_self_signed_trust_precedes_identity_and_signs_only_final_bundles(self):
        leaf_bytes = b"mock leaf certificate"
        self.policy["certificate_sha1"] = ci.hashlib.sha1(leaf_bytes).hexdigest()
        self.policy["certificate_sha256"] = ci.hashlib.sha256(leaf_bytes).hexdigest()
        calls = []
        def extract(_p12, _password, private):
            pem, der = private / "leaf.pem", private / "leaf.der"
            pem.write_bytes(b"pem")
            der.write_bytes(leaf_bytes)
            self.assertEqual((_p12.stat().st_mode & 0o777), 0o600)
            return pem, der
        def run(args, **_kwargs):
            args = [str(arg) for arg in args]
            calls.append(args)
            if args[1] == "find-identity":
                return '  1) ' + self.policy["certificate_sha1"] + ' "Mock identity"'
            return ""
        with mock.patch.object(ci, "_configured_policy", return_value=self.policy), \
             mock.patch.dict(os.environ, {"MACOS_DEV_CERT_P12_BASE64": "cDEy", "MACOS_DEV_CERT_PASSWORD": "secret"}), \
             mock.patch.object(ci, "_extract_leaf", side_effect=extract), \
             mock.patch.object(ci, "_run", side_effect=run), \
             mock.patch.object(ci, "_execute", return_value=ci.subprocess.CompletedProcess([], 0, "", "")), \
             mock.patch.object(ci.signing, "verify_app", return_value={"valid": True}), \
             mock.patch("builtins.print"):
            ci.sign()
        trust = next(index for index, args in enumerate(calls) if "add-trusted-cert" in args)
        identity = next(index for index, args in enumerate(calls) if "find-identity" in args)
        self.assertLess(trust, identity)
        codesign = [args for args in calls if args[0] == "codesign"]
        self.assertEqual(len(codesign), 3)
        self.assertTrue(all(args[-1].endswith("ScreenHop Dev.app") and "--keychain" in args for args in codesign))
        self.assertFalse(any(args[0] == "cargo" for args in calls))
        self.assertFalse(ci._private_dir().exists())

    def test_archive_roundtrip_policy_failure_deletes_zip(self):
        output = self.root / "mock.zip"
        def run(args, **_kwargs):
            if "-c" in args:
                with ci.zipfile.ZipFile(output, "w") as archive:
                    archive.writestr("ScreenHop Dev.app/Contents/mock", b"mock")
            return ""
        with mock.patch.object(ci.signing, "verify_app", side_effect=[{"valid": True}, RuntimeError("invalid extracted bundle")]), \
             mock.patch.object(ci, "_run", side_effect=run):
            with self.assertRaisesRegex(RuntimeError, "invalid extracted bundle"):
                ci._archive_one(self.root / "ScreenHop Dev.app", output, self.policy, ["arm64"])
        self.assertFalse(output.exists())
        self.assertFalse((self.root / "mock.extracted").exists())

    def test_archive_rejects_changed_record_before_zip(self):
        with mock.patch.object(ci.signing, "verify_app", return_value={"new": True}), \
             mock.patch.object(ci, "_run") as run:
            with self.assertRaisesRegex(RuntimeError, "签名步骤记录不同"):
                ci._archive_one(self.root / "ScreenHop Dev.app", self.root / "mock.zip", self.policy, ["arm64"], {"old": True})
        run.assert_not_called()

    def test_archive_rejects_invalid_bundle_before_zip(self):
        _, attempt = ci._attempt_root()
        attempt.mkdir(parents=True)
        (attempt / "signing-reports.json").write_text(json.dumps({"arm64": {}, "x86_64": {}, "universal": {}}))
        with mock.patch.object(ci, "_configured_policy", return_value=self.policy), \
             mock.patch.object(ci.signing, "verify_app", side_effect=RuntimeError("invalid")), \
             mock.patch.object(ci, "_run") as run:
            with self.assertRaisesRegex(RuntimeError, "invalid"):
                ci.archive(output_dir=self.root / "out")
            self.assertFalse(any("ditto" in str(call) for call in run.call_args_list))
            self.assertFalse(list((self.root / "out").glob("*.zip")) if (self.root / "out").exists() else [])


if __name__ == "__main__":
    unittest.main()
