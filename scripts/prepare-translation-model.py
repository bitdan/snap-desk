"""固定 HF revision 转换 INT8 包，并用原生 Worker 与 HF 逐项校验分词/解码。

只由开发/CI 调用，不在客户端安装 Python。所有校验成功才发布输出目录。
"""
import argparse
import hashlib
import json
import re
import shutil
import subprocess
import tempfile
from pathlib import Path

FILES = {"model.bin", "config.json", "shared_vocabulary.json", "source_vocabulary.json",
         "target_vocabulary.json", "source.spm", "target.spm", "LICENSE.txt"}
SAMPLES = ["Hello world!", "Settings saved.", "你好，世界！", "  Save   changes?  ",
           "Line one\nLine two", "😀 café 中文 English", "1234567890 {name} %s Ctrl+Shift+V",
           "https://example.com/a?q=1", r"C:\Users\Example\file.txt", "“Quoted” — words…"]


def sha256(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def prepare(args):
    # 延迟导入让 --help 和语法检查不要求下载大型转换依赖。
    import ctranslate2
    from huggingface_hub import snapshot_download
    from transformers import MarianTokenizer

    if ctranslate2.__version__ != "4.6.0":
        raise ValueError("请使用 translation-requirements.txt 中的固定依赖")
    if not re.fullmatch(r"[a-f0-9]{40}", args.revision):
        raise ValueError("--revision 必须是 40 位 HF commit，不允许 main/tag")
    if args.output.exists():
        raise ValueError("输出目录已存在；请选择新目录，避免覆盖已验证包")
    repo = "Helsinki-NLP/opus-mt-" + args.direction
    source = snapshot_download(repo_id=repo, revision=args.revision,
                               allow_patterns=["*.json", "*.spm", "pytorch_model.bin", "*.safetensors", "README.md"])
    tokenizer = MarianTokenizer.from_pretrained(source, local_files_only=True)
    token = ">>cmn_Hans<<" if args.direction == "en-zh" else ""
    if token and token not in tokenizer.get_vocab():
        raise ValueError("源模型词表缺少简体中文目标标签")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=args.output.parent) as staging:
        pack_dir = Path(staging) / "pack"
        ctranslate2.converters.TransformersConverter(source).convert(str(pack_dir), quantization="int8")
        for name in ["source.spm", "target.spm"]:
            shutil.copyfile(Path(source) / name, pack_dir / name)
        # 许可由开发者从固定源版本取出审核，不编造许可正文或省略署名。
        shutil.copyfile(args.license_file, pack_dir / "LICENSE.txt")
        actual = {p.name for p in pack_dir.iterdir()}
        if actual - FILES or not {"model.bin", "config.json", "source.spm", "target.spm", "LICENSE.txt"} <= actual:
            raise ValueError(f"转换产物与 Worker 契约不匹配：{actual}")
        pack = dict(modelId=f"opus-mt-{args.direction}-int8", sourceRevision=args.revision,
                    runtime="4.6.0", tokenizerProfile="transformers-marian-spm-v1",
                    targetLanguageToken=token, license="Apache-2.0" if token else "CC-BY-4.0", files=[])
        for path in sorted(pack_dir.iterdir()):
            pack["files"].append(dict(name=path.name, size=path.stat().st_size, sha256=sha256(path),
                                      url=f"{args.base_url.rstrip('/')}/{path.name}" if args.base_url else ""))
        (pack_dir / "pack.json").write_text(json.dumps(pack, ensure_ascii=False, indent=2), encoding="utf-8")
        validate_worker(args.worker.resolve(), pack_dir.resolve(), pack, tokenizer, token)
        # rename 在同一文件系统内提交；校验失败临时目录自动清理，旧输出不受影响。
        pack_dir.rename(args.output)
    print(f"已验证原生分词/解码及固定样本翻译：{args.output}")
    print("发布下载前仍需人工评估双向翻译质量、许可和断网截图流程。")


def validate_worker(executable, directory, pack, tokenizer, token):
    """每次请求有硬超时；失败终止进程，不把部分校验结果作为成功包。"""
    import queue
    import threading
    process = subprocess.Popen([str(executable)], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.DEVNULL, encoding="utf-8", text=True,
                               creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
    responses = queue.Queue()

    def read():
        while True:
            line = process.stdout.readline(256 * 1024 + 1)
            responses.put(line)
            if not line:
                return

    threading.Thread(target=read, daemon=True).start()
    count = 0

    def call(action, **kwargs):
        nonlocal count
        count += 1
        request = dict(id=str(count), protocol=1, action=action, **kwargs)
        process.stdin.write(json.dumps(request, ensure_ascii=False) + "\n")
        process.stdin.flush()
        line = responses.get(timeout=120)
        if len(line) > 256 * 1024 or not line.endswith("\n"):
            raise ValueError("Worker 响应超限或提前退出")
        response = json.loads(line)
        if response.get("id") != str(count) or response.get("protocol") != 1 or not response.get("ok"):
            raise ValueError(f"Worker 协议或推理失败：{response}")
        return response

    try:
        call("health")
        call("load", directory=str(directory), modelId=pack["modelId"])
        for text in SAMPLES:
            expected = tokenizer.convert_ids_to_tokens(tokenizer.encode(token + text))
            actual = call("tokenize", text=text)["tokens"]
            if actual != expected:
                raise ValueError(f"源分词不一致：{text!r}\nHF={expected}\nWorker={actual}")
            target_ids = tokenizer(text_target=text)["input_ids"]
            target_tokens = tokenizer.convert_ids_to_tokens(target_ids)
            reference = tokenizer.decode(target_ids, skip_special_tokens=True, clean_up_tokenization_spaces=False)
            decoded = call("decode", tokens=target_tokens)["text"]
            if decoded != reference:
                raise ValueError(f"目标解码不一致：{text!r}\nHF={reference!r}\nWorker={decoded!r}")
        sample = "Hello world!" if token else "你好，世界！"
        result = call("translate", segments=[dict(id="0", text=sample)])["items"]
        if len(result) != 1 or result[0]["id"] != "0" or not result[0]["text"].strip():
            raise ValueError("原生翻译样本为空或段落映射错误")
        # 保留受控固定样本供人工验收；不读取用户截图或原文。
        print(f"Golden translation: {sample} -> {result[0]['text']}")
    finally:
        process.kill()
        process.wait(timeout=10)
        process.stdin.close()
        process.stdout.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--direction", choices=["en-zh", "zh-en"], required=True)
    parser.add_argument("--revision", required=True)
    parser.add_argument("--worker", type=Path, required=True)
    parser.add_argument("--license-file", type=Path, required=True, help="审核后的模型许可全文及署名")
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--base-url", default="", help="可选：该版本已发布的 HTTPS 文件目录")
    arguments = parser.parse_args()
    if arguments.base_url and not arguments.base_url.startswith("https://"):
        parser.error("下载地址必须使用 HTTPS")
    prepare(arguments)
