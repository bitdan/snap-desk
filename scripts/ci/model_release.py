"""复核模型文件及编译时目录；与 Rust Pack 契约同步，不执行模型包内代码。"""
import hashlib
import json
import re
import time
import urllib.error
import urllib.request
from pathlib import Path
from urllib.parse import urlsplit

FILES = {"model.bin", "config.json", "shared_vocabulary.json", "source_vocabulary.json",
         "target_vocabulary.json", "source.spm", "target.spm", "LICENSE.txt"}
REQUIRED = {"model.bin", "config.json", "source.spm", "target.spm", "LICENSE.txt"}
REPOSITORY = "bitdan/snap-desk"


def validate(pack, tag=None, repository=REPOSITORY):
    if set(pack) != {"modelId", "sourceRevision", "runtime", "tokenizerProfile", "targetLanguageToken", "license", "files"}:
        raise ValueError("pack 字段与客户端契约不一致")
    direction = {"opus-mt-en-zh-int8": "en-zh", "opus-mt-zh-en-int8": "zh-en"}.get(pack["modelId"])
    if not direction or not re.fullmatch(r"[0-9a-f]{40}", pack["sourceRevision"]):
        raise ValueError("模型身份或 HF revision 无效")
    if (pack["runtime"], pack["tokenizerProfile"], pack["targetLanguageToken"], pack["license"]) != (
            "4.6.0", "transformers-marian-spm-v1", ">>cmn_Hans<<" if direction == "en-zh" else "",
            "Apache-2.0" if direction == "en-zh" else "CC-BY-4.0"):
        raise ValueError("模型 runtime/分词器/许可契约无效")
    names, tags, total = set(), set(), 0
    for item in pack["files"]:
        if set(item) != {"name", "size", "sha256", "url"}:
            raise ValueError("文件字段无效")
        name, size = item["name"], item["size"]
        if name not in FILES or name in names or type(size) is not int or not 0 < size <= 1024 ** 3:
            raise ValueError("文件名、大小或重复条目无效")
        if not re.fullmatch(r"[0-9a-f]{64}", item["sha256"]):
            raise ValueError("SHA-256 无效")
        url = urlsplit(item["url"])
        prefix = f"/{repository}/releases/download/"
        parts = url.path.removeprefix(prefix).split("/")
        if url.scheme != "https" or url.netloc != "github.com" or url.query or url.fragment or not url.path.startswith(prefix) or len(parts) != 2:
            raise ValueError("必须使用固定仓库 GitHub Release URL")
        expected = f"translation-model-{direction}-{pack['sourceRevision'][:12]}-r[1-9][0-9]*"
        if not re.fullmatch(expected, parts[0]) or parts[1] != name or (tag and parts[0] != tag):
            raise ValueError("附件版本/文件名不匹配")
        names.add(name)
        tags.add(parts[0])
        total += size
    if len(tags) != 1 or total > 1536 * 1024 ** 2 or not REQUIRED <= names or not (
            "shared_vocabulary.json" in names or {"source_vocabulary.json", "target_vocabulary.json"} <= names):
        raise ValueError("文件集合不完整、版本混用或总大小超限")
    return pack


def read_pack(directory, tag=None, repository=REPOSITORY):
    path = directory / "pack.json"
    if path.stat().st_size > 64 * 1024:
        raise ValueError("pack.json 大小超限")
    pack = validate(json.loads(path.read_text(encoding="utf-8")), tag, repository)
    if {p.name for p in directory.iterdir()} != {"pack.json", *(f["name"] for f in pack["files"])}:
        raise ValueError("模型目录有未声明文件")
    for item in pack["files"]:
        path = directory / item["name"]
        if path.is_symlink() or not path.is_file() or path.stat().st_size != item["size"]:
            raise ValueError(f"文件类型/大小无效: {path}")
        with path.open("rb") as stream:
            if hashlib.file_digest(stream, "sha256").hexdigest() != item["sha256"]:
                raise ValueError(f"摘要不匹配: {path}")
    return pack


class ReleaseRedirect(urllib.request.HTTPRedirectHandler):
    """匿名附件请求只允许 HTTPS GitHub CDN，拒绝降级及任意站点跳转。"""
    max_redirections = 3
    max_repeats = 3

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        url = urlsplit(newurl)
        if url.scheme != "https" or url.netloc != "release-assets.githubusercontent.com":
            raise ValueError("模型附件跳转到不受信任的目标")
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def verify_remote(pack):
    opener = urllib.request.build_opener(ReleaseRedirect())
    for item in pack["files"]:
        # 刚发布的附件可能短暂 404/5xx；从头重试网络失败，不重试摘要/大小/跳转违规。
        for attempt in range(3):
            try:
                size, sha = 0, hashlib.sha256()
                with opener.open(item["url"], timeout=60) as response:
                    if response.status != 200:
                        raise ValueError("附件 HTTP 状态异常")
                    while chunk := response.read(1024 * 1024):
                        size += len(chunk)
                        if size > item["size"]:
                            raise ValueError("附件下载超出声明大小")
                        sha.update(chunk)
                if size != item["size"] or sha.hexdigest() != item["sha256"]:
                    raise ValueError("已发布附件与审核清单不一致")
                break
            except (urllib.error.URLError, TimeoutError, ConnectionError) as error:
                if isinstance(error, urllib.error.HTTPError) and error.code not in {404, 429, 500, 502, 503, 504}:
                    raise
                if attempt == 2:
                    raise
                time.sleep(2 ** (attempt + 1))
