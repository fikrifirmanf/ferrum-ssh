"""Exercise packaging control flow without certificates or Apple network access."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parent
MOCK = '''#!/usr/bin/env python3
import json, os, pathlib, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ["CALLS"], "a") as f:
    f.write(json.dumps([name, *args]) + "\\n")
if name == "cargo":
    p = pathlib.Path("target/release/ferrum")
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text("binary")
if name == "xcrun" and args[:2] == ["notarytool", "submit"] and os.environ.get("REJECT"):
    sys.exit(1)
if name == "iconutil":
    pathlib.Path(args[args.index("-o") + 1]).touch()
if name in ("ditto", "hdiutil"):
    pathlib.Path(args[-1]).touch()
'''


class MacPackagingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        shutil.copytree(SOURCE, self.root / "packaging", ignore=shutil.ignore_patterns("__pycache__"))
        (self.root / "Cargo.toml").write_text('[package]\nversion = "1.2.3"\n')
        self.bin = self.root / "bin"
        self.bin.mkdir()
        for name in ("cargo", "codesign", "xcrun", "spctl", "iconutil", "ditto", "hdiutil", "security"):
            p = self.bin / name
            p.write_text(MOCK)
            p.chmod(0o755)
        self.env = {k: v for k, v in os.environ.items()
                    if not k.startswith(("MACOS_", "APPLE_", "CIRCLE_"))}
        self.env.update(PATH=f"{self.bin}:{os.environ['PATH']}", CALLS=str(self.root / "calls"))

    def run_build(self, *, signed=False, ci=False, **env):
        if signed:
            self.env.update(MACOS_SIGNING_IDENTITY="Developer ID Application: Ferrum (TEAM)",
                            MACOS_NOTARY_PROFILE="test-profile")
        self.env.update(env)
        script = "build-macos-ci.sh" if ci else "build-macos.sh"
        result = subprocess.run(["sh", str(self.root / "packaging" / script)],
                                env=self.env, capture_output=True, text=True)
        calls_path = self.root / "calls"
        self.calls = [json.loads(line) for line in calls_path.read_text().splitlines()] if calls_path.exists() else []
        return result

    def ci_credentials(self):
        return dict(MACOS_CERTIFICATE_P12_BASE64="dGVzdA==",
                    MACOS_CERTIFICATE_PASSWORD="test-password",
                    APPLE_ID="developer@example.test", APPLE_TEAM_ID="TESTTEAM",
                    APPLE_APP_SPECIFIC_PASSWORD="test-notary-password")

    def test_ci_imports_certificate_and_deletes_temporary_keychain(self):
        result = self.run_build(signed=True, ci=True, **self.ci_credentials())
        self.assertEqual(result.returncode, 0, result.stderr)
        create = next(c for c in self.calls if c[:2] == ["security", "create-keychain"])
        keychain = create[-1]
        self.assertIn(["security", "delete-keychain", keychain], self.calls)
        self.assertFalse(Path(keychain).parent.exists())
        submissions = [c for c in self.calls if c[:3] == ["xcrun", "notarytool", "submit"]]
        self.assertEqual(len(submissions), 2)
        self.assertEqual(submissions[0][-2:], ["--keychain", keychain])
        self.assertIn("ferrum-ci", submissions[0])
        self.assertFalse(any(c[:2] == ["security", "list-keychains"] for c in self.calls))

    def test_ci_deletes_credentials_when_notarization_fails(self):
        result = self.run_build(signed=True, ci=True, REJECT="1", **self.ci_credentials())
        self.assertNotEqual(result.returncode, 0)
        create = next(c for c in self.calls if c[:2] == ["security", "create-keychain"])
        self.assertIn(["security", "delete-keychain", create[-1]], self.calls)
        self.assertFalse(Path(create[-1]).parent.exists())

    def test_tag_without_credentials_falls_back_to_adhoc(self):
        result = self.run_build(ci=True, CIRCLE_TAG="v1.2.3")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("local ad-hoc build", result.stderr)
        self.assertIn(["codesign", "--force", "--sign", "-", "dist/Ferrum.app"], self.calls)
        self.assertFalse(any(c[0] in ("xcrun", "security") for c in self.calls))
        self.assertEqual(len(list((self.root / "dist").glob("*.dmg"))), 1)

    def test_required_notarization_without_credentials_fails_before_build(self):
        result = self.run_build(ci=True, CIRCLE_TAG="v1.2.3", MACOS_REQUIRE_NOTARIZATION="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Developer ID Application", result.stderr)
        self.assertEqual(self.calls, [])

    def test_partial_signing_configuration_fails(self):
        result = self.run_build(MACOS_SIGNING_IDENTITY="Developer ID Application: Ferrum (TEAM)")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("MACOS_NOTARY_PROFILE", result.stderr)
        self.assertEqual(self.calls, [])

    def test_adhoc_identity_cannot_be_used_for_release(self):
        result = self.run_build(MACOS_SIGNING_IDENTITY="-", MACOS_NOTARY_PROFILE="test")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls, [])

    def test_local_build_warns_and_does_not_notarize(self):
        result = self.run_build()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("local ad-hoc build", result.stderr)
        self.assertIn(["codesign", "--force", "--sign", "-", "dist/Ferrum.app"], self.calls)
        self.assertFalse(any(c[0] == "xcrun" for c in self.calls))
        self.assertEqual(len(list((self.root / "dist").glob("*.dmg"))), 1)

    def test_release_staples_app_before_dmg_and_validates_both(self):
        result = self.run_build(signed=True, MACOS_SIGNING_KEYCHAIN="/tmp/test.keychain-db")
        self.assertEqual(result.returncode, 0, result.stderr)
        app_staple = self.calls.index(["xcrun", "stapler", "staple", "dist/Ferrum.app"])
        create = next(i for i, c in enumerate(self.calls) if c[0] == "hdiutil")
        self.assertLess(app_staple, create)
        submissions = [c for c in self.calls if c[:3] == ["xcrun", "notarytool", "submit"]]
        self.assertEqual(len(submissions), 2)
        self.assertTrue(submissions[0][3].endswith(".zip"))
        self.assertTrue(submissions[1][3].endswith(".dmg"))
        for call in submissions:
            self.assertEqual(call[4:], ["--keychain-profile", "test-profile", "--wait", "--keychain", "/tmp/test.keychain-db"])
        signatures = [c for c in self.calls if c[0] == "codesign" and "--sign" in c]
        self.assertEqual(len(signatures), 2)
        self.assertIn("--options", signatures[0])
        self.assertIn("runtime", signatures[0])
        for call in signatures:
            self.assertIn("--timestamp", call)
            self.assertNotIn("-", call)
        self.assertEqual(len([c for c in self.calls if c[:3] == ["xcrun", "stapler", "validate"]]), 2)
        self.assertEqual(len([c for c in self.calls if c[:2] == ["spctl", "--assess"]]), 2)

    def test_notarization_failure_prevents_dmg_creation(self):
        result = self.run_build(signed=True, REJECT="1")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any(c[0] == "hdiutil" for c in self.calls))
        submission = next(c[3] for c in self.calls if c[:3] == ["xcrun", "notarytool", "submit"])
        self.assertFalse(Path(submission).parent.exists(), "Temporary signing files must be removed")


if __name__ == "__main__":
    unittest.main()
