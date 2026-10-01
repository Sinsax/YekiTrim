#!/usr/bin/env python3
"""EmoteLab 贴纸重命名工具：备份 -> 原地改名。

原始格式: ysyk_<内容>_<时间戳>.<ext>    (时间戳: YYYY-MM-DD-HH-MM-SS)
目标格式: <内容>(空格转下划线).<ext>
  例: ysyk_爱心 1_2026-07-23-17-26-12.gif  ->  爱心_1.gif
      ysyk_PNGTuber 说话_2026-07-23-17-27-40.gif -> PNGTuber_说话.gif
      ysyk_被击中(爱心)_2026-07-23-17-24-11.gif   ->  被击中(爱心).gif

流程:
  1. 备份: 先把所有匹配的原件复制到 <src>/backup/ (已存在则跳过, 不覆盖)
  2. 改名: 原地重命名 (去 ysyk_ 前缀与时间戳, 内容名空格转下划线)
  3. 冲突: 目标名已存在则追加 _2/_3 后缀 (同贴纸重复下载时保留全部文件)

用法:
  python scripts/rename_emotelab_stickers.py [--src <目录>] [--dry-run] [--no-backup]
"""
import argparse
import re
import shutil
from pathlib import Path

# ysyk_<内容>_<时间戳>   (内容贪婪匹配, 时间戳锚定结尾)
PATTERN = re.compile(r"^ysyk_(.+)_(\d{4}-\d{2}-\d{2}-\d{2}-\d{2}-\d{2})$")


def normalize_content(content: str) -> str:
    """内容名归一化: 空格转下划线 (帧号 ' 1' 一并转为 '_1'), 去除首尾空白"""
    return re.sub(r"\s+", "_", content.strip())


def main() -> int:
    ap = argparse.ArgumentParser(description="EmoteLab 贴纸重命名工具: 备份 -> 原地改名")
    ap.add_argument("--src", default=r"F:\\para\\Code\\V-Trim\\release\\vtrim-v0.2.1-win-x64\\stickers\\ysyk_0.9.0",
                    help="贴纸目录 (默认 target\\debug\\stickers)")
    ap.add_argument("--dry-run", action="store_true", help="只预览改动, 不改任何文件")
    ap.add_argument("--no-backup", action="store_true", help="跳过备份步骤")
    args = ap.parse_args()

    src = Path(args.src)
    if not src.exists():
        print(f"[ERROR] 目录不存在: {src}")
        return 1

    # 1. 收集匹配的源文件 (仅根目录一层, 不递归)
    sources = []  # (源文件, 归一化内容名)
    skipped = 0
    for f in sorted(src.iterdir()):
        if not f.is_file():
            continue
        m = PATTERN.match(f.stem)
        if m:
            sources.append((f, normalize_content(m.group(1))))
        else:
            skipped += 1

    if not sources:
        print("  没有匹配的 ysyk_* 文件 (可能已处理过 — 幂等, 无需再跑)")
        return 0

    # 2. 分配目标名: 避开根目录现有文件 + 本次已分配的名字
    used = {p.name for p in src.iterdir() if p.is_file()}
    targets = []  # (源文件, 目标文件名)
    for f, content in sources:
        base, suffix = content, f.suffix
        cand = f"{base}{suffix}"
        n = 1
        while cand in used:
            n += 1
            cand = f"{base}_{n}{suffix}"
        used.add(cand)
        targets.append((f, cand))

    # 3. 备份
    backup = src / "backup"
    if not args.no_backup:
        backup.mkdir(exist_ok=True)
        for f, _ in targets:
            b = backup / f.name
            if b.exists():
                print(f"  [SKIP] 备份已存在: {b.name}")
                continue
            if args.dry_run:
                print(f"  [DRY]  备份: {f.name} -> backup/{f.name}")
            else:
                shutil.copy2(f, b)
                print(f"  [BK]   {f.name} -> backup/{f.name}")

    # 4. 原地改名
    for f, cand in targets:
        if args.dry_run:
            print(f"  [DRY]  {f.name} -> {cand}")
        else:
            f.rename(src / cand)
            print(f"  [OK]   {f.name} -> {cand}")

    print(f"\n--- 完成 ---")
    print(f"  改名: {len(targets)}   跳过(不匹配): {skipped}")
    if not args.dry_run:
        print(f"  备份目录: {backup}")
    if not args.no_backup and not args.dry_run:
        print("  提示: 若存在旧中间目录 renamed/ 请手动删除")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
