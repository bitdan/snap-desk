"""不编译、不下载大模型的发布契约回归测试。"""
import copy
import hashlib
import importlib.util
import io
import json
import os
import stat
import struct
import subprocess
import sys
import tempfile
import unittest
import urllib.request
import urllib.error
import zipfile
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import MagicMock, patch
from model_release import read_pack, validate, ReleaseRedirect
import model_release

spec = importlib.util.spec_from_file_location("runtime", Path(__file__).with_name("translator-runtime.py"))
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


class ModelTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.directory = Path(self.temp.name)
        self.tag = "translation-model-en-zh-aaaaaaaaaaaa-r1"
        self.pack = dict(modelId="opus-mt-en-zh-int8", sourceRevision="a" * 40, runtime="4.6.0",
                         tokenizerProfile="transformers-marian-spm-v1", targetLanguageToken=">>cmn_Hans<<",
                         license="Apache-2.0", files=[])
        for name in ("model.bin", "config.json", "source.spm", "target.spm", "LICENSE.txt", "shared_vocabulary.json"):
            data = name.encode()
            (self.directory / name).write_bytes(data)
            self.pack["files"].append(dict(name=name, size=len(data), sha256=hashlib.sha256(data).hexdigest(),
                                           url=f"https://github.com/bitdan/snap-desk/releases/download/{self.tag}/{name}"))
        (self.directory / "pack.json").write_text(json.dumps(self.pack), encoding="utf-8")

    def test_complete_pack_and_tampered_file(self):
        self.assertEqual(read_pack(self.directory, self.tag), self.pack)
        (self.directory / "model.bin").write_bytes(b"corrupted")
        with self.assertRaises(ValueError):
            read_pack(self.directory, self.tag)

    def test_extra_executable_rejected(self):
        (self.directory / "worker.exe").write_bytes(b"x")
        with self.assertRaises(ValueError):
            read_pack(self.directory, self.tag)

    def test_invalid_contracts(self):
        changes = [lambda p: p["files"].append(p["files"][0]),
                   lambda p: p["files"][0].update(name="../model.bin"),
                   lambda p: p["files"][0].update(size=2 ** 30 + 1),
                   lambda p: p["files"][0].update(url="https://github.com.evil.test/model.bin"),
                   lambda p: p["files"][0].update(url=p["files"][0]["url"].replace("-r1/", "-r2/")),
                   lambda p: p.update(targetLanguageToken=""),
                   lambda p: p.update(sourceRevision="main"),
                   lambda p: p["files"].pop()]
        for change in changes:
            with self.subTest(change=change):
                pack = copy.deepcopy(self.pack)
                change(pack)
                with self.assertRaises(ValueError):
                    validate(pack)


class RuntimeTests(unittest.TestCase):
    def test_cmake_cache_blank_lines_and_comments_do_not_corrupt_keys(self):
        for newline in ("\n", "\r\n"):
            with self.subTest(newline=newline):
                text = newline.join(["# CMake cache", "", *(
                    f"//Build option{newline}{key}:STRING={value}{newline}"
                    for key, value in runtime.OPTIONS.items()),
                    "CMAKE_CXX_COMPILER:FILEPATH=C:/Program Files/MSVC/cl.exe",
                    "EXTRA:STRING=a=b", "EMPTY:STRING="])
                cache = runtime.parse_cmake_cache(text)
                runtime.validate_ct2_options(cache)
                self.assertEqual(cache["CMAKE_CXX_COMPILER"], "C:/Program Files/MSVC/cl.exe")
                self.assertEqual(cache["EXTRA"], "a=b")
                self.assertEqual(cache["EMPTY"], "")
                self.assertTrue(all("\n" not in key for key in cache))

    def test_cmake_option_mismatch_reports_actual_and_missing_values(self):
        cache = dict(runtime.OPTIONS, WITH_CUDA="ON")
        del cache["WITH_RUY"]
        with self.assertRaises(ValueError) as error:
            runtime.validate_ct2_options(cache)
        self.assertIn("WITH_CUDA: expected='OFF', actual='ON'", str(error.exception))
        self.assertIn("WITH_RUY: expected='ON', actual='<missing>'", str(error.exception))

    def test_license_collection_preserves_nested_notices_and_skips_git(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            names = ["LICENSE", "third_party/ruy/LICENSE", "third_party/ruy/AUTHORS",
                     "third_party/ruy/NOTICE.txt"]
            for name in [*names, ".git/LICENSE", "src/main.cpp"]:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture", encoding="utf-8")
            self.assertEqual(runtime.license_files(root), sorted(root / name for name in names))

    def test_license_collection_does_not_follow_windows_links(self):
        # 模拟 runner 的不可访问链接，以及不是 symlink 的 Windows junction。
        link = MagicMock(name="inaccessible_cub")
        link.name = "cub"
        link.is_symlink.return_value = True
        link.stat.side_effect = PermissionError("link target denied")
        junction = MagicMock(name="junction")
        junction.name = "junction"
        junction.is_symlink.return_value = False
        junction.stat.return_value = SimpleNamespace(
            st_mode=stat.S_IFDIR, st_file_attributes=stat.FILE_ATTRIBUTE_REPARSE_POINT)
        with patch.object(runtime.os, "scandir") as scan:
            scan.return_value.__enter__.return_value = [link, junction]
            self.assertEqual(runtime.license_files(Path("source")), [])
            scan.assert_called_once()
        link.stat.assert_not_called()
        junction.stat.assert_called_once_with(follow_symlinks=False)

    def test_license_collection_does_not_hide_real_permission_errors(self):
        entry = MagicMock(name="unreadable_license")
        entry.name = "LICENSE"
        entry.is_symlink.return_value = False
        entry.stat.side_effect = PermissionError("real file denied")
        with patch.object(runtime.os, "scandir") as scan:
            scan.return_value.__enter__.return_value = [entry]
            with self.assertRaises(PermissionError):
                runtime.license_files(Path("source"))

    def test_zip_traversal_and_duplicate_rejected(self):
        for names in [("../worker.exe",), ("/worker.exe",), ("C:/worker.exe",), ("A.dll", "a.dll")]:
            with self.subTest(names=names), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                archive = root / "runtime.zip"
                with zipfile.ZipFile(archive, "w") as stream:
                    for name in names:
                        stream.writestr(name, b"x")
                with self.assertRaises(ValueError):
                    runtime.extract(SimpleNamespace(archive=archive, directory=root / "out"))

    def test_pe_import_and_delay_import(self):
        # 最小 PE32+ fixture 同时含常规与 delay-load DLL，避免只验证普通依赖。
        data = bytearray(2048)
        data[:2] = b"MZ"
        struct.pack_into("<I", data, 0x3C, 0x80)
        data[0x80:0x84] = b"PE\0\0"
        struct.pack_into("<HH", data, 0x84, 0x8664, 1)
        struct.pack_into("<H", data, 0x94, 240)
        struct.pack_into("<H", data, 0x98, 0x20B)
        struct.pack_into("<IIII", data, 0x188 + 8, 0x600, 0x1000, 0x600, 0x200)
        struct.pack_into("<I", data, 0x98 + 112 + 8, 0x1000)
        struct.pack_into("<I", data, 0x98 + 112 + 13 * 8, 0x1080)
        struct.pack_into("<I", data, 0x200 + 12, 0x1100)
        struct.pack_into("<II", data, 0x280, 1, 0x1120)
        data[0x300:0x30D] = b"kernel32.dll\0"
        data[0x320:0x32A] = b"delay.dll\0"
        with tempfile.TemporaryDirectory() as temp:
            path = Path(temp) / "worker.exe"
            path.write_bytes(data)
            self.assertEqual(runtime.pe_dependencies(path), ["delay.dll", "kernel32.dll"])
            struct.pack_into("<H", data, 0x84, 0x14C)
            path.write_bytes(data)
            with self.assertRaises(ValueError):
                runtime.pe_dependencies(path)


class RedirectTests(unittest.TestCase):
    def test_remote_download_retries_temporary_404(self):
        data = b"model fixture"
        response = io.BytesIO(data)
        response.status = 200
        opener = MagicMock()
        opener.open.side_effect = [urllib.error.HTTPError("https://example.test", 404, "not ready", {}, None), response]
        pack = {"files": [dict(url="https://example.test", size=len(data), sha256=hashlib.sha256(data).hexdigest())]}
        with patch.object(model_release.urllib.request, "build_opener", return_value=opener), \
             patch.object(model_release.time, "sleep") as sleep:
            model_release.verify_remote(pack)
            sleep.assert_called_once_with(2)
        self.assertEqual(opener.open.call_count, 2)

    def test_remote_download_rejects_corruption_without_retry(self):
        for expected_size, expected_hash in [(1, "0" * 64), (3, "0" * 64)]:
            with self.subTest(size=expected_size):
                response = io.BytesIO(b"bad")
                response.status = 200
                opener = MagicMock()
                opener.open.return_value = response
                pack = {"files": [dict(url="https://example.test", size=expected_size, sha256=expected_hash)]}
                with patch.object(model_release.urllib.request, "build_opener", return_value=opener), \
                     patch.object(model_release.time, "sleep") as sleep:
                    with self.assertRaises(ValueError):
                        model_release.verify_remote(pack)
                    sleep.assert_not_called()
                opener.open.assert_called_once()

    def test_only_observed_https_cdn_is_allowed(self):
        handler = ReleaseRedirect()
        request = urllib.request.Request("https://github.com/bitdan/snap-desk/releases/download/tag/model.bin")
        valid = "https://release-assets.githubusercontent.com/file?signature=example"
        self.assertEqual(handler.redirect_request(request, None, 302, "Found", {}, valid).full_url, valid)
        for url in ("http://release-assets.githubusercontent.com/file", "https://127.0.0.1/file",
                    "https://release-assets.githubusercontent.com.evil.test/file",
                    "https://user:secret@release-assets.githubusercontent.com/file"):
            with self.subTest(url=url), self.assertRaises(ValueError):
                handler.redirect_request(request, None, 302, "Found", {}, url)


class ConsoleEncodingTests(unittest.TestCase):
    def test_runtime_verify_success_output_under_redirected_windows_encoding(self):
        # 执行真实 verify() 到最后一行；仅替代原生 PE 扫描与 Worker 进程，避免编译。
        # 子进程使用管道输出，复现 runner 而不是本地交互控制台的编码行为。
        code = r'''
import importlib.util, json, sys, tempfile
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch
spec = importlib.util.spec_from_file_location("runtime", sys.argv[1])
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)
with tempfile.TemporaryDirectory() as temp:
    directory = Path(temp)
    files = []
    for name in ["snap-translator-worker.exe", "licenses/ctranslate2/LICENSE",
                 "licenses/sentencepiece/LICENSE", "licenses/json/LICENSE"]:
        path = directory / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(b"test fixture")
        files.append(dict(name=name, size=path.stat().st_size, sha256=runtime.digest(path)))
    info = dict(tag="translation-engine-v4.6.0-r1", runtime="4.6.0", protocol=1,
                target="windows-x64", options=runtime.OPTIONS, dependencies={}, files=files)
    (directory / "runtime-build.json").write_text(json.dumps(info), encoding="utf-8")
    health = SimpleNamespace(stdout=json.dumps(dict(protocol=1, id="health", ok=True, runtime="4.6.0")))
    with patch.object(runtime, "dependency_graph", return_value={}), \
         patch.object(runtime.subprocess, "run", return_value=health), \
         patch.dict(runtime.os.environ, {"SystemRoot": "C:/Windows"}):
        runtime.verify(SimpleNamespace(directory=directory, tag=info["tag"]))
'''
        script = str(Path(__file__).with_name("translator-runtime.py").resolve())
        env = dict(os.environ, PYTHONUTF8="0", PYTHONIOENCODING="cp1252")
        broken = subprocess.run([sys.executable, "-c", code, script], env=env, capture_output=True)
        self.assertNotEqual(broken.returncode, 0)
        self.assertIn(b"UnicodeEncodeError", broken.stderr)
        # 与三个工作流及两个 PowerShell 入口一致，同时固定 UTF-8 模式和标准流编码。
        env.update(PYTHONUTF8="1", PYTHONIOENCODING="utf-8")
        fixed = subprocess.run([sys.executable, "-c", code, script], env=env, capture_output=True)
        self.assertEqual(fixed.returncode, 0, fixed.stderr.decode("utf-8"))
        self.assertIn("已验证 translation-engine-v4.6.0-r1", fixed.stdout.decode("utf-8"))
        self.assertIn("SHA-256 / health", fixed.stdout.decode("utf-8"))

    def test_catalog_cli_can_print_chinese_in_ci(self):
        root = Path(__file__).resolve().parents[2]
        env = dict(os.environ, PYTHONUTF8="1", PYTHONIOENCODING="utf-8")
        with tempfile.TemporaryDirectory() as temp:
            catalog = Path(temp) / "catalog.json"
            catalog.write_text("[]", encoding="utf-8")
            result = subprocess.run([sys.executable, "scripts/ci/generate-model-catalog.py",
                                     "--check", "--output", str(catalog)],
                                    cwd=root, env=env, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr.decode("utf-8"))
        self.assertIn("已验证 0 个模型清单", result.stdout.decode("utf-8"))


if __name__ == "__main__":
    unittest.main()
