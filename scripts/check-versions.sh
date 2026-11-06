#!/usr/bin/env bash
# Сверка версии в четырёх местах, где она объявлена независимо.
#
# Расхождение здесь не ломает сборку — оно её портит молча: пакет .deb получит
# одну версию, окно «О программе» другую, а витрина третью. Найти это потом
# можно только у пользователя, поэтому проверка стоит до сборки, а не после.
#
# Аргумент (необязательно) — ожидаемая версия, обычно из метки: v0.1.0 или
# 0.1.0. Без аргумента просто сверяет файлы между собой.
set -euo pipefail

cd "$(dirname "$0")/.."

fail() { echo "ОШИБКА: $*" >&2; exit 1; }

json_version() {
  # Без jq: он есть не в каждом образе, а поле здесь одно и предсказуемое.
  python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"])' "$1"
}

pkg=$(json_version package.json)
tauri=$(json_version src-tauri/tauri.conf.json)
cargo=$(awk '/^\[package\]/{p=1;next} /^\[/{p=0} p && /^version *=/{gsub(/[" ]/,"");sub(/version=/,"");print;exit}' src-tauri/Cargo.toml)

[ -n "$cargo" ] || fail "не нашёл version в src-tauri/Cargo.toml"

echo "package.json      $pkg"
echo "tauri.conf.json   $tauri"
echo "Cargo.toml        $cargo"

[ "$pkg" = "$tauri" ] || fail "package.json ($pkg) и tauri.conf.json ($tauri) разошлись"
[ "$pkg" = "$cargo" ] || fail "package.json ($pkg) и Cargo.toml ($cargo) разошлись"

# Журнал изменений обязателен на всех трёх языках: релиз без описания — это
# файл без объяснения, что в нём изменилось, а описание только на одном языке
# превращается в релиз с дырой для остальных. Заголовок ищем точный, чтобы
# «похожая» строка не сошла за раздел.
for log in CHANGELOG.md CHANGELOG_ru.md CHANGELOG_uz.md; do
  grep -qE "^## \[$pkg\] — " "$log" \
    || fail "в $log нет раздела '## [$pkg] — ДАТА'"
  printf '%-17s раздел [%s] на месте\n' "$log" "$pkg"
done

# Пустой аргумент — это «метки нет» (сборка ветки), а не «метка пустая».
if [ $# -ge 1 ] && [ -n "$1" ]; then
  want=${1#v}
  # Предварительная метка v0.1.1-rc1 относится к версии 0.1.1 в файлах.
  want=${want%%-rc[0-9]*}
  [ "$want" = "$pkg" ] || fail "метка v$want не совпадает с версией в файлах ($pkg)"
  echo "метка             v$want совпадает"
fi

echo "Версии сходятся: $pkg"
