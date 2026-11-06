#!/usr/bin/env bash
# Проверка scripts/release-notes.sh и scripts/check-versions.sh на подставных
# журналах: сборка описания релиза из трёх языков не должна молча терять язык.
#
# Работает во временном каталоге и настоящие файлы не трогает.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fails=0
ok()   { echo "ок:      $*"; }
bad()  { echo "ПРОВАЛ:  $*" >&2; fails=$((fails + 1)); }

fixture() {
  mkdir -p "$tmp/scripts" "$tmp/src-tauri"
  cp "$root/scripts/release-notes.sh" "$root/scripts/check-versions.sh" "$tmp/scripts/"
  cat > "$tmp/CHANGELOG.md" <<'EOT'
# Changelog

## [Unreleased]

### Added
- not released yet

## [0.2.0] — 2026-12-01

Second release.

### Added
- **English feature.**

## [0.1.0] — 2026-11-01

First.

<!-- trailer -->
EOT
  cat > "$tmp/CHANGELOG_ru.md" <<'EOT'
# Изменения

## [Не выпущено]

## [0.2.0] — 2026-12-01

Второй выпуск.

### Добавлено
- **Русская возможность.**

## [0.1.0] — 2026-11-01

Первый.
EOT
  cat > "$tmp/CHANGELOG_uz.md" <<'EOT'
# O'zgarishlar

## [Chiqarilmagan]

## [0.2.0] — 2026-12-01

Ikkinchi chiqarish.

### Qo'shildi
- **O'zbekcha imkoniyat.**

## [0.1.0] — 2026-11-01

Birinchi.
EOT
  echo '{"version":"0.2.0"}' > "$tmp/package.json"
  echo '{"version":"0.2.0"}' > "$tmp/src-tauri/tauri.conf.json"
  printf '[package]\nname = "x"\nversion = "0.2.0"\n' > "$tmp/src-tauri/Cargo.toml"
}

run() { (cd "$tmp" && "$@"); }

fixture

# 1. Тело релиза: английский, затем русский и узбекский — каждый только своей версии.
out=$(run scripts/release-notes.sh 0.2.0)
printf '%s' "$out" | head -1 | grep -qx 'Second release.' && ok "начинается с английского" || bad "не начинается с английского"
en_at=$(printf '%s\n' "$out" | grep -n 'English feature' | cut -d: -f1)
ru_at=$(printf '%s\n' "$out" | grep -n '^## Русский$' | cut -d: -f1)
uz_at=$(printf '%s\n' "$out" | grep -n '^## O‘zbekcha$' | cut -d: -f1)
[ -n "$en_at" ] && [ -n "$ru_at" ] && [ -n "$uz_at" ] && [ "$en_at" -lt "$ru_at" ] && [ "$ru_at" -lt "$uz_at" ] \
  && ok "порядок: английский, русский, узбекский" || bad "порядок разделов нарушен"
printf '%s' "$out" | grep -q 'Русская возможность' && printf '%s' "$out" | grep -q "O'zbekcha imkoniyat" \
  && ok "тексты всех трёх языков на месте" || bad "потерян текст одного из языков"
printf '%s' "$out" | grep -qE 'not released yet|First\.|Первый|Birinchi|trailer' \
  && bad "в описание попало чужое: другая версия, «Не выпущено» или хвост" || ok "чужого в описании нет"

# 2. v-префикс допустим.
run scripts/release-notes.sh v0.2.0 >/dev/null && ok "метка с v принимается" || bad "метка с v не принята"

# 3. Нет раздела в одном из файлов — ошибка с понятным сообщением и пустым выводом.
for f in CHANGELOG.md CHANGELOG_ru.md CHANGELOG_uz.md; do
  fixture
  python3 - "$tmp/$f" <<'PY'
import re, sys
p = sys.argv[1]
s = open(p).read()
s = re.sub(r"## \[0\.2\.0\] — 2026-12-01.*?(?=## \[0\.1\.0\])", "", s, flags=re.S)
open(p, "w").write(s)
PY
  set +e
  stdout=$(run scripts/release-notes.sh 0.2.0 2>"$tmp/err")
  code=$?
  set -e
  if [ "$code" -ne 0 ] && [ -z "$stdout" ] && grep -q "$f" "$tmp/err"; then
    ok "без раздела в $f — ошибка, и файл назван в сообщении"
  else
    bad "без раздела в $f не было ошибки с именем файла (код $code)"
  fi
  set +e
  run scripts/check-versions.sh >/dev/null 2>"$tmp/err"
  code=$?
  set -e
  if [ "$code" -ne 0 ] && grep -q "$f" "$tmp/err"; then
    ok "check-versions ловит отсутствие раздела в $f"
  else
    bad "check-versions пропустил отсутствие раздела в $f"
  fi
done

# 4. Все три на месте — check-versions проходит.
fixture
run scripts/check-versions.sh v0.2.0 >/dev/null && ok "check-versions проходит при полном комплекте" || bad "check-versions упал при полном комплекте"

# 5. Предварительная метка v0.2.0-rc1 относится к версии 0.2.0; чужая версия — нет.
run scripts/check-versions.sh v0.2.0-rc1 >/dev/null && ok "метка -rc1 сверяется с версией 0.2.0" || bad "метка -rc1 не принята"
if run scripts/check-versions.sh v0.3.0-rc1 >/dev/null 2>&1; then bad "метка v0.3.0-rc1 принята при версии 0.2.0"; else ok "чужая метка -rc отвергается"; fi

[ "$fails" -eq 0 ] || { echo "Провалов: $fails" >&2; exit 1; }
echo "Всё сошлось."
