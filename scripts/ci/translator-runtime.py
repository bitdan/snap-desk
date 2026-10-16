"""Windows runtime 的归档和验收；仅使用标准库，不依赖模型转换环境。"""
import argparse
import hashlib
import json
import os
import re
import shutil
import stat
import struct
import subprocess
import zipfile
from pathlib import Path

OPTIONS = dict(WITH_CUDA="OFF", WITH_MKL="OFF", WITH_DNNL="OFF", WITH_OPENBLAS="OFF",
               WITH_RUY="ON", OPENMP_RUNTIME="NONE", BUILD_SHARED_LIBS="ON",
               CMAKE_MSVC_RUNTIME_LIBRARY="MultiThreaded")
# 仅放行 Windows 自带组件；VC/OpenMP 等必须随包提供，不能借用 runner 的 PATH。
SYSTEM_DLLS = set("kernel32.dll kernelbase.dll ntdll.dll advapi32.dll bcrypt.dll crypt32.dll "
                 "user32.dll gdi32.dll ole32.dll oleaut32.dll shell32.dll shlwapi.dll "
                 "ws2_32.dll rpcrt4.dll secur32.dll version.dll winmm.dll powrprof.dll "
                 "psapi.dll userenv.dll ucrtbase.dll".split())


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def pe_dependencies(path):
    """读取 x64 PE import 和 delay-import 表；递归检查由调用者完成。"""
    data = path.read_bytes()
    def u16(offset):
        return struct.unpack_from("<H", data, offset)[0]
    def u32(offset):
        return struct.unpack_from("<I", data, offset)[0]
    if data[:2] != b"MZ":
        raise ValueError(f"不是 PE 文件: {path}")
    pe = u32(0x3C)
    if data[pe:pe + 4] != b"PE\0\0" or u16(pe + 4) != 0x8664:
        raise ValueError(f"不是 Windows x64: {path}")
    optional = pe + 24
    if u16(optional) != 0x20B:
        raise ValueError("需要 PE32+")
    sections = optional + u16(pe + 20)
    def offset(rva):
        for index in range(u16(pe + 6)):
            pos = sections + 40 * index
            size, address, raw_size, raw = struct.unpack_from("<IIII", data, pos + 8)
            if address <= rva < address + max(size, raw_size):
                return raw + rva - address
        raise ValueError(f"非法 PE RVA: {path}")
    result = set()
    for index, width, name_pos in [(1, 20, 12), (13, 32, 4)]:
        rva = u32(optional + 112 + index * 8)
        if not rva:
            continue
        pos = offset(rva)
        while any(data[pos:pos + width]):
            if index == 13 and u32(pos) != 1:
                raise ValueError("不支持 VA 格式 delay-import")
            name = offset(u32(pos + name_pos))
            dll = data[name:data.index(b"\0", name)].decode("ascii").lower()
            if not re.fullmatch(r"[a-z0-9_.-]+\.dll", dll):
                raise ValueError("非法 DLL 名称")
            result.add(dll)
            pos += width
    return sorted(result)


def dependency_graph(directory, search=()):
    available = {p.name.lower(): p for p in directory.glob("*.dll")}
    candidates = {p.name.lower(): p for root in search for p in root.rglob("*.dll")}
    pending = [directory / "snap-translator-worker.exe", *available.values()]
    graph = {}
    while pending:
        path = pending.pop()
        if path.name in graph:
            continue
        dependencies = pe_dependencies(path)
        graph[path.name] = dependencies
        for name in dependencies:
            if name in SYSTEM_DLLS or name.startswith(("api-ms-win-", "ext-ms-win-")):
                continue
            if name not in available:
                if name not in candidates:
                    raise ValueError(f"缺少非系统依赖: {path.name} -> {name}")
                target = directory / name
                shutil.copyfile(candidates[name], target)
                available[name] = target
            pending.append(available[name])
    return graph


def git_sha(path):
    return subprocess.check_output(["git", "-C", str(path), "rev-parse", "HEAD"], text=True).strip()


def license_files(source):
    """遍历真实源码目录，不跟随链接；实际目录/许可的读取错误仍然使 CI 失败。"""
    notices = []
    pending = [source]
    while pending:
        with os.scandir(pending.pop()) as entries:
            for entry in entries:
                if entry.name == ".git":
                    continue
                # Windows 上游目录可能含无法访问目标的链接（如 thrust/cub）。
                # 先识别链接，再用 no-follow stat；不能先 is_file() 访问链接目标。
                if entry.is_symlink():
                    continue
                metadata = entry.stat(follow_symlinks=False)
                if getattr(metadata, "st_file_attributes", 0) & stat.FILE_ATTRIBUTE_REPARSE_POINT:
                    continue
                path = Path(entry.path)
                if stat.S_ISDIR(metadata.st_mode):
                    pending.append(path)
                elif stat.S_ISREG(metadata.st_mode) and entry.name.upper().startswith(
                        ("LICENSE", "COPYING", "NOTICE", "AUTHORS", "COPYRIGHT")):
                    notices.append(path)
    return sorted(notices)


def prepare(args):
    directory = args.directory
    manifest = directory / "runtime-build.json"
    if manifest.exists():
        raise ValueError("拒绝覆盖已有 runtime manifest")
    graph = dependency_graph(directory, [args.ct2_install])
    sources = {"ctranslate2": args.ct2_source,
               "sentencepiece": args.worker_build / "_deps/sentencepiece-src",
               "json": args.worker_build / "_deps/json-src"}
    commits = {}
    for name, source in sources.items():
        commits[name] = git_sha(source)
        # 保留上游相对路径，包含 Ruy/嵌套第三方的 LICENSE/COPYING/NOTICE/AUTHORS。
        notices = license_files(source)
        if not notices:
            raise ValueError(f"缺少上游许可: {name}")
        for path in notices:
            target = directory / "licenses" / name / path.relative_to(source)
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(path, target)
    if not list((directory / "licenses/ctranslate2/third_party/ruy").rglob("LICENSE*")):
        raise ValueError("未收集 Ruy 许可，请检查递归子模块")
    caches = {}
    for name, build in [("ct2", args.ct2_install.parent / "ct2-build"), ("worker", args.worker_build)]:
        cache = (build / "CMakeCache.txt").read_text(encoding="utf-8")
        caches[name] = {m[0]: m[1] for m in re.findall(r"^([^/#][^:=]*):[^=]+=(.*)$", cache, re.M)}
    if any(caches["ct2"].get(key) != value for key, value in OPTIONS.items()):
        raise ValueError("CT2 实际构建选项与 CPU/RUY 契约不一致")
    info = dict(tag=args.tag, runtime="4.6.0", protocol=1, target="windows-x64",
                repositorySha=git_sha(Path.cwd()), sources=commits, options=OPTIONS,
                ct2Submodules=subprocess.check_output(["git", "-C", str(args.ct2_source), "submodule", "status", "--recursive"], text=True).splitlines(),
                cmakeVersion=subprocess.check_output(["cmake", "--version"], text=True).strip(),
                toolchain={name: {k: v for k, v in cache.items() if k.startswith(("CMAKE_CXX_COMPILER", "CMAKE_GENERATOR", "CMAKE_VS_PLATFORM_TOOLSET"))}
                           for name, cache in caches.items()}, dependencies=graph,
                files=[dict(name=p.relative_to(directory).as_posix(), size=p.stat().st_size, sha256=digest(p))
                       for p in sorted(directory.rglob("*")) if p.is_file()])
    manifest.write_text(json.dumps(info, indent=2) + "\n", encoding="utf-8")


def verify(args):
    directory = args.directory.resolve()
    info = json.loads((directory / "runtime-build.json").read_text(encoding="utf-8"))
    for key, expected in dict(tag=args.tag, runtime="4.6.0", protocol=1, target="windows-x64", options=OPTIONS).items():
        if info.get(key) != expected:
            raise ValueError(f"runtime 契约不匹配: {key}")
    actual = {p.relative_to(directory).as_posix() for p in directory.rglob("*") if p.is_file()}
    names = [item["name"] for item in info["files"]]
    if len(names) != len(set(names)) or set(names) != actual - {"runtime-build.json"}:
        raise ValueError("runtime 文件清单不完整或含多余文件")
    for item in info["files"]:
        path = directory / item["name"]
        if path.is_symlink() or path.stat().st_size != item["size"] or digest(path) != item["sha256"]:
            raise ValueError(f"runtime 文件校验失败: {path}")
    for source in ("ctranslate2", "sentencepiece", "json"):
        if not any(n.startswith(f"licenses/{source}/") for n in names):
            raise ValueError(f"缺少许可: {source}")
    if dependency_graph(directory) != info["dependencies"]:
        raise ValueError("DLL 依赖图不匹配")
    # 清空开发环境 PATH；协议/退出码/超时同时检查，不能把启动即崩溃当健康。
    env = {key: value for key, value in os.environ.items() if key.upper() in {"SYSTEMROOT", "WINDIR", "TEMP", "TMP"}}
    env["PATH"] = str(Path(os.environ["SystemRoot"]) / "System32")
    result = subprocess.run([str(directory / "snap-translator-worker.exe")],
                            input='{"protocol":1,"id":"health","action":"health"}\n',
                            capture_output=True, text=True, encoding="utf-8", timeout=30,
                            cwd=directory, env=env, check=True)
    response = json.loads(result.stdout)
    if any(response.get(k) != v for k, v in dict(protocol=1, id="health", ok=True, runtime="4.6.0").items()):
        raise ValueError("Worker health 失败")
    print(f"已验证 {args.tag}: x64 / DLL / 许可 / SHA-256 / health")


def extract(args):
    if args.directory.exists():
        raise ValueError("runtime 目标目录必须不存在")
    with zipfile.ZipFile(args.archive) as archive:
        seen = set()
        if sum(p.file_size for p in archive.infolist()) > 512 * 1024 * 1024:
            raise ValueError("runtime 解压大小超限")
        for item in archive.infolist():
            name = item.filename.replace("\\", "/").rstrip("/")
            if not name or any(p in {"", ".", ".."} for p in name.split("/")) or ":" in name or name.lower() in seen:
                raise ValueError("ZIP 成员路径非法或重复")
            seen.add(name.lower())
        archive.extractall(args.directory)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    for command in ("prepare", "verify"):
        sub = commands.add_parser(command)
        sub.add_argument("directory", type=Path)
        sub.add_argument("--tag", required=True)
        if command == "prepare":
            for option in ("ct2-source", "ct2-install", "worker-build"):
                sub.add_argument("--" + option, type=Path, required=True)
    sub = commands.add_parser("extract")
    sub.add_argument("archive", type=Path)
    sub.add_argument("directory", type=Path)
    args = parser.parse_args()
    globals()[args.command](args)
