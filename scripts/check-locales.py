#!/usr/bin/env python3
"""Сверка локалей: ключи одинаковы во всех шести и все используемые существуют.

Заведён после того, как кнопка «Сохранить» показала человеку сырой ключ
`common.save`: его не было ни в одной локали. Компилятор такого не видит —
`t("common.save")` это строка, — а увидеть должен был бы пользователь.

Проверяется две вещи:

1. **Наборы ключей во всех локалях совпадают.** Расхождение означает, что
   часть языков молча показывает английский запасной текст или сам ключ.
2. **Каждый ключ из `t("...")` существует.** Обратное (ключ есть, но не
   используется) — не ошибка: тексты бывают заготовлены заранее.

Запуск: python3 scripts/check-locales.py
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOCALES = ROOT / "src" / "locales"
LANGS = ["uz", "ru", "en", "kk", "ky", "tr"]


def keys_of(path: Path) -> set[str]:
    """Плоский набор ключей из файла локали.

    Разбор по отступам, а не полноценным парсером TypeScript: файлы пишутся
    одним стилем — два пробела на уровень, — и полный парсер здесь был бы
    дороже задачи.
    """
    keys: set[str] = set()
    stack: list[str] = []

    # Длинный текст prettier переносит на следующую строку: `key:` и уже под
    # ним строка в кавычках. Склеиваем такие пары обратно — иначе разбор
    # объявляет ключ отсутствующим, хотя он есть. Проверено на
    # `tofu.mitmWarning`, где это и вылезло.
    raw = path.read_text(encoding="utf-8").splitlines()
    lines: list[str] = []
    skip = False
    for i, line in enumerate(raw):
        if skip:
            skip = False
            continue
        if re.match(r'^\s*[a-zA-Z_][\w]*:$', line) and i + 1 < len(raw):
            nxt = raw[i + 1].lstrip()
            if nxt.startswith(('"', '`')):
                lines.append(line.rstrip() + ' ' + nxt)
                skip = True
                continue
        lines.append(line)

    for line in lines:
        stripped = line.strip()
        if not stripped or stripped.startswith("//"):
            continue
        indent = (len(line) - len(line.lstrip())) // 2
        opened = re.match(r'^([a-zA-Z_][\w]*):\s*\{$', stripped)
        leaf = re.match(r'^([a-zA-Z_][\w]*):\s*["`]', stripped)
        if opened:
            stack = stack[: indent - 1] + [opened.group(1)]
        elif leaf:
            path_ = stack[: indent - 1] + [leaf.group(1)]
            keys.add(".".join(path_))
        elif stripped.startswith("}"):
            stack = stack[: max(indent - 1, 0)]
    return keys


def used_keys() -> dict[str, str]:
    """Ключи из `t("...")` вместе с файлом, где встретились."""
    found: dict[str, str] = {}
    for f in list((ROOT / "src").rglob("*.vue")) + list((ROOT / "src").rglob("*.ts")):
        text = f.read_text(encoding="utf-8")
        # Обе формы кавычек: в разметке пишут t('...'), в скриптах t("...").
        # Проверка, видящая только одну, пропускает половину вызовов — и
        # пропустила: `agentSudoReadPlaceholder` в одинарных кавычках она не
        # заметила, а ключ лежал не в том разделе.
        for m in re.finditer(r"""\bt\(\s*["']([a-zA-Z][\w.]*)["']""", text):
            found.setdefault(m.group(1), str(f.relative_to(ROOT)))
    return found


def main() -> int:
    sets = {lang: keys_of(LOCALES / f"{lang}.ts") for lang in LANGS}
    base = sets["ru"]
    problems: list[str] = []

    for lang in LANGS:
        missing = base - sets[lang]
        extra = sets[lang] - base
        for k in sorted(missing):
            problems.append(f"{lang}: нет ключа {k} (есть в ru)")
        for k in sorted(extra):
            problems.append(f"{lang}: лишний ключ {k} (нет в ru)")

    for key, where in sorted(used_keys().items()):
        if key not in base:
            problems.append(f"{where}: используется t(\"{key}\"), которого нет в локалях")

    if problems:
        print("Локали разошлись:\n")
        for p in problems:
            print("  •", p)
        print(f"\nВсего: {len(problems)}. Человек увидит сырой ключ вместо текста.")
        return 1

    print(f"Локали сошлись: {len(base)} ключей в каждом из {len(LANGS)} языков.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
