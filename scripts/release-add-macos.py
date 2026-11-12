#!/usr/bin/env python3
"""Добавить сборку macOS в файлы уже вышедшего выпуска.

Linux и Windows собирает GitLab и сам делает releases.json, latest.json и
SHA256SUMS. macOS собирается отдельно, на GitHub, и позже. Этот скрипт
дописывает в те же три файла записи для Mac, а не заводит четвёртый: витрина и
обновления читают ровно по одному файлу, и склеивать их им незачем.

Использование:
  release-add-macos.py ВЕРСИЯ БАЗОВЫЙ_URL КАТАЛОГ

В КАТАЛОГЕ лежат файлы выпуска (releases.json, latest.json, SHA256SUMS — каких
нет, те создаются) и сборка Mac: Darwaza_<версия>_universal.dmg,
Darwaza_<версия>_universal.app.tar.gz и .app.tar.gz.sig. Повторный запуск
заменяет прежние записи Mac, а не дублирует их.
"""
import hashlib
import json
import pathlib
import sys
from urllib.parse import quote


def sha256(path: pathlib.Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> None:
    if len(sys.argv) != 4:
        sys.exit(__doc__)
    version = sys.argv[1].lstrip("v")
    base = sys.argv[2].rstrip("/")
    root = pathlib.Path(sys.argv[3])

    dmg = root / f"Darwaza_{version}_universal.dmg"
    archive = root / f"Darwaza_{version}_universal.app.tar.gz"
    sig = root / f"{archive.name}.sig"
    for p in (dmg, archive, sig):
        if not p.is_file():
            sys.exit(f"ОШИБКА: нет {p}")

    # SHA256SUMS: строки Mac заменяем, остальное не трогаем.
    sums_path = root / "SHA256SUMS"
    mac_names = {dmg.name, archive.name}
    lines = []
    if sums_path.exists():
        for line in sums_path.read_text().splitlines():
            parts = line.split(maxsplit=1)
            if len(parts) == 2 and parts[1].lstrip("*") in mac_names:
                continue
            lines.append(line)
    for p in (dmg, archive):
        lines.append(f"{sha256(p)}  {p.name}")
    sums_path.write_text("\n".join(lines) + "\n")

    # releases.json — для витрины: человеку нужен .dmg.
    rel_path = root / "releases.json"
    if rel_path.exists():
        releases = json.loads(rel_path.read_text())
        if releases.get("version") != version:
            sys.exit(f"ОШИБКА: releases.json от версии {releases.get('version')}, а сборка {version}")
    else:
        releases = {"version": version, "releasedAt": None, "assets": []}
    releases["assets"] = [a for a in releases.get("assets", []) if a.get("platform") != "macos"]
    releases["assets"].append({
        "platform": "macos",
        "kind": "dmg",
        "url": f"{base}/{quote(dmg.name)}",
        "size": dmg.stat().st_size,
        "sha256": sha256(dmg),
    })
    rel_path.write_text(json.dumps(releases, ensure_ascii=False, indent=2) + "\n")

    # latest.json — для обновлений. Сборка одна на оба процессора, поэтому обе
    # записи указывают на один архив с одной подписью.
    latest_path = root / "latest.json"
    if latest_path.exists():
        latest = json.loads(latest_path.read_text())
        if latest.get("version") != version:
            sys.exit(f"ОШИБКА: latest.json от версии {latest.get('version')}, а сборка {version}")
    else:
        latest = {"version": version, "pub_date": releases.get("releasedAt"), "platforms": {}}
    entry = {"signature": sig.read_text().strip(), "url": f"{base}/{quote(archive.name)}"}
    for key in ("darwin-aarch64", "darwin-x86_64"):
        latest["platforms"][key] = entry
    latest_path.write_text(json.dumps(latest, ensure_ascii=False, indent=2) + "\n")

    print(f"macOS добавлен в выпуск {version}: {dmg.name}, {archive.name}")


if __name__ == "__main__":
    main()
