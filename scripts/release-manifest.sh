#!/usr/bin/env bash
# Собирает готовые пакеты в один каталог и описывает их:
#   SHA256SUMS    — для человека, проверяется одной командой sha256sum -c
#   releases.json — для витрины, ровно тот формат, который читает
#                   `GET /downloads/latest` (RELEASES_FILE на сервере)
#
# Контрольные суммы обязательны: витрина обещает проверку целостности, и
# обещание без эталона — обман. Оба файла делает один и тот же проход, чтобы
# они не могли разойтись между собой.
#
# Использование:
#   scripts/release-manifest.sh ВЕРСИЯ БАЗОВЫЙ_URL [КАТАЛОГ_ВЫВОДА]
# Пример:
#   scripts/release-manifest.sh 0.1.0 https://gitlab.example/api/v4/.../0.1.0
set -euo pipefail

cd "$(dirname "$0")/.."

[ $# -ge 2 ] || { echo "использование: $0 ВЕРСИЯ БАЗОВЫЙ_URL [КАТАЛОГ]" >&2; exit 2; }
version=${1#v}
base_url=${2%/}
out=${3:-dist-release}

bundle=${BUNDLE_DIR:-src-tauri/target/release/bundle}
[ -d "$bundle" ] || { echo "ОШИБКА: нет каталога сборок $bundle" >&2; exit 1; }

rm -rf "$out"
mkdir -p "$out"

# Порядок в списке — порядок на витрине. deb первым: это самый частый Linux.
#
# Windows и macOS (#160, #161) собираются на своих машинах и кладут файлы в тот
# же каталог: манифест собирается один на все системы, иначе витрине пришлось
# бы склеивать три файла и решать, какой из них главный.
mapfile -t files < <(
  find "$bundle" -maxdepth 2 -type f \
    \( -name '*.deb' -o -name '*.rpm' -o -name '*.AppImage' \
       -o -name '*.msi' -o -name '*-setup.exe' -o -name '*.dmg' \) | sort
)

[ ${#files[@]} -gt 0 ] || { echo "ОШИБКА: в $bundle нет ни одного пакета (.deb/.rpm/.AppImage/.msi/.exe/.dmg)" >&2; exit 1; }

for f in "${files[@]}"; do cp "$f" "$out/"; done

released_at=${RELEASED_AT:-$(date -u +%Y-%m-%dT%H:%M:%SZ)}

(cd "$out" && sha256sum -- * > SHA256SUMS)
# Сама сумма в список сумм попасть не может.
(cd "$out" && grep -v ' SHA256SUMS$' SHA256SUMS > .sums && mv .sums SHA256SUMS)

VERSION="$version" BASE_URL="$base_url" RELEASED_AT="$released_at" OUT="$out" python3 - <<'PY'
import json, os, pathlib
from urllib.parse import quote

out = pathlib.Path(os.environ['OUT'])
base = os.environ['BASE_URL']

kinds = {'.deb': 'deb', '.rpm': 'rpm', '.AppImage': 'AppImage',
         '.msi': 'msi', '.exe': 'exe', '.dmg': 'dmg'}
# Система нужна витрине, чтобы предложить человеку его файл, а не список из
# шести. Определяется по расширению: другого признака в имени файла нет.
platforms = {'deb': 'linux', 'rpm': 'linux', 'AppImage': 'linux',
             'msi': 'windows', 'exe': 'windows', 'dmg': 'macos'}
order = {'deb': 0, 'rpm': 1, 'AppImage': 2, 'msi': 3, 'exe': 4, 'dmg': 5}

sums = {}
for line in (out / 'SHA256SUMS').read_text().splitlines():
    digest, name = line.split(maxsplit=1)
    sums[name.lstrip('*')] = digest

assets = []
for name, digest in sums.items():
    kind = kinds.get(pathlib.Path(name).suffix)
    if kind is None:
        continue
    assets.append({
        'platform': platforms[kind],
        'kind': kind,
        # Имя файла в адресе кодируем: у пакетов оно с пробелами не бывает,
        # но полагаться на это нельзя — productName может стать двухсловным.
        'url': f'{base}/{quote(name)}',
        'size': (out / name).stat().st_size,
        'sha256': digest,
    })

assets.sort(key=lambda a: order[a['kind']])

manifest = {
    'version': os.environ['VERSION'],
    'releasedAt': os.environ['RELEASED_AT'],
    'assets': assets,
}
(out / 'releases.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + '\n')
PY

echo "Готово: $out"
ls -1 "$out"

# ── Манифест обновлений (#164) ───────────────────────────────────────────
#
# Отдельный файл рядом с releases.json: витрине нужен список файлов для
# человека, а приложению — версия и подпись. Один файл на две задачи потом
# ломается при первой же правке под одну из них.
#
# Формат задан Tauri: `version`, `pub_date`, `platforms.<цель>.{signature,url}`.
# Подпись создаётся при сборке рядом с пакетом, файлом `<пакет>.sig`. Нет
# подписи — значит сборка не подписана, и манифест не пишем: приложение всё
# равно отвергнет такое обновление, а пустой манифест выглядел бы как рабочий.
# AppImage предпочитаем осознанно: именно его приложение умеет заменить собой
# при обновлении. Пакеты deb и rpm тоже подписываются, но ставит и обновляет
# их менеджер пакетов системы, а не мы. Порядок поиска здесь — не мелочь:
# `find` с несколькими -name выдаёт файлы в порядке обхода каталога, то есть
# как повезёт.
sig_file="$(find "$bundle" -name '*.AppImage.sig' | head -1)"
[ -n "$sig_file" ] || sig_file="$(find "$bundle" -name '*.deb.sig' | head -1)"
if [ -n "$sig_file" ]; then
  pkg="$(basename "${sig_file%.sig}")"
  cat > "$out/latest.json" <<JSON
{
  "version": "$version",
  "pub_date": "$released_at",
  "platforms": {
    "linux-x86_64": {
      "signature": "$(cat "$sig_file")",
      "url": "$base_url/$pkg"
    }
  }
}
JSON
  echo "манифест обновлений: $out/latest.json"
else
  echo "ВНИМАНИЕ: подписи не найдено — latest.json не создан." >&2
  echo "Авто-обновление на эту сборку работать не будет." >&2
fi
