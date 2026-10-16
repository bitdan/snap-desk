"""不编译、不下载大模型的发布契约回归测试。"""
import copy
import hashlib
import importlib.util
import json
import struct
import tempfile
import unittest
import urllib.request
import zipfile
from pathlib import Path
from types import SimpleNamespace
from model_release import read_pack, validate, ReleaseRedirect

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


if __name__ == "__main__":
    unittest.main()
