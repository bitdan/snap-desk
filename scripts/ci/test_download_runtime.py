"""真实运行 PowerShell 下载入口；仅替代 gh 下载和原生 health，不访问网络。"""
import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path

SHELL = shutil.which("pwsh") or shutil.which("powershell")
BASH = shutil.which("bash") if os.name != "nt" else None
if os.name == "nt" and shutil.which("git"):
    git_bash = Path(shutil.which("git")).parent.parent / "bin/bash.exe"
    if git_bash.is_file():
        BASH = str(git_bash)


@unittest.skipUnless(BASH, "GNU bash/sha256sum required")
class ChecksumWorkflowTests(unittest.TestCase):
    def test_real_publish_checksum_command_accepts_crlf_but_rejects_bad_hash(self):
        workflow = Path(__file__).resolve().parents[2] / ".github/workflows/build-translation-engine.yml"
        # 直接运行实际工作流命令，防止测试副本正确但 YAML 又退回不兼容写法。
        command = next(line.strip() for line in workflow.read_text(encoding="utf-8").splitlines()
                       if "sha256sum --check" in line)
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            data = b"checksum fixture"
            (root / "translator-runtime-windows-x64.zip").write_bytes(data)
            for label, ending, valid in [("LF", b"\n", True), ("CRLF", b"\r\n", True), ("corrupt", b"\r\n", False)]:
                with self.subTest(label=label):
                    digest = hashlib.sha256(data).hexdigest() if valid else "0" * 64
                    (root / "translator-runtime-windows-x64.zip.sha256").write_bytes(
                        (digest + "  translator-runtime-windows-x64.zip").encode("ascii") + ending)
                    result = subprocess.run([BASH, "-c", "set -euo pipefail\n" + command], cwd=root, capture_output=True)
                    self.assertEqual(result.returncode == 0, valid, repr(result.stderr))


@unittest.skipUnless(SHELL and os.name == "nt", "Windows PowerShell integration test")
class DownloadRuntimeTests(unittest.TestCase):
    def test_install_failure_cleanup_and_retry(self):
        with tempfile.TemporaryDirectory(prefix="runtime-download-") as temp:
            root = Path(temp)
            scripts = root / "scripts"
            scripts.mkdir()
            source = Path(__file__).parent
            for name in ["download-translator-runtime.ps1", "translator-runtime.py"]:
                shutil.copyfile(source / name, scripts / name)
            archive = root / "fixture.zip"
            with zipfile.ZipFile(archive, "w") as stream:
                stream.writestr("snap-translator-worker.exe", b"test fixture")
            lock = dict(tag="translation-engine-v4.6.0-r1", sha256=hashlib.sha256(archive.read_bytes()).hexdigest(),
                        runtime="4.6.0", protocol=1, target="windows-x64", asset="translator-runtime-windows-x64.zip")
            lock_path = scripts / "translation-engine.json"
            lock_path.write_text(json.dumps(lock), encoding="utf-8")
            verify = scripts / "verify-translator-runtime.ps1"
            destination = root / "application" / "translator-runtime"
            env = dict(os.environ, TEST_ARCHIVE=str(archive), TEST_SCRIPT=str(scripts / "download-translator-runtime.ps1"),
                       TEST_DESTINATION=str(destination), GH_REPO="example/test")
            command = r'''
$ErrorActionPreference = 'Stop'
function gh {
    $index = [Array]::IndexOf($args, '--dir')
    Copy-Item -LiteralPath $env:TEST_ARCHIVE -Destination (Join-Path $args[$index + 1] 'translator-runtime-windows-x64.zip')
    $global:LASTEXITCODE = 0
}
& $env:TEST_SCRIPT -RuntimeDir $env:TEST_DESTINATION
'''
            def run():
                return subprocess.run([SHELL, "-NoProfile", "-NonInteractive", "-Command", command],
                                      env=env, capture_output=True)
            # 验收失败不遗留最终目录，也不遗留临时下载；随后无需手动删除即可重试。
            verify.write_text("param($RuntimeDir, $EngineTag)\nthrow 'fixture health failed'\n", encoding="utf-8")
            self.assertNotEqual(run().returncode, 0)
            self.assertFalse(destination.exists())
            self.assertEqual(list(destination.parent.iterdir()), [])
            # 正确 ZIP 但锁摘要错误，同样不得进入验收或安装。
            lock_path.write_text(json.dumps(dict(lock, sha256="0" * 64)), encoding="utf-8")
            self.assertNotEqual(run().returncode, 0)
            self.assertEqual(list(destination.parent.iterdir()), [])
            lock_path.write_text(json.dumps(lock), encoding="utf-8")
            verify.write_text("param($RuntimeDir, $EngineTag)\nif (!(Test-Path (Join-Path $RuntimeDir 'snap-translator-worker.exe'))) { throw 'missing worker' }\n", encoding="utf-8")
            result = run()
            self.assertEqual(result.returncode, 0, repr(result.stderr))
            self.assertEqual((destination / "snap-translator-worker.exe").read_bytes(), b"test fixture")
            self.assertEqual(list(destination.parent.iterdir()), [destination])


if __name__ == "__main__":
    unittest.main()
