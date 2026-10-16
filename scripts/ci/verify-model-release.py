"""发布前复核本地模型包，可选匿名校验发布后的附件。"""
import argparse
from pathlib import Path
from model_release import read_pack, verify_remote

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--expected-tag", required=True)
    parser.add_argument("--repository", default="bitdan/snap-desk")
    parser.add_argument("--remote", action="store_true")
    args = parser.parse_args()
    pack = read_pack(args.directory, args.expected_tag, args.repository)
    if args.remote:
        verify_remote(pack)
    print("模型文件、版本 URL、大小和 SHA-256 验证通过")
