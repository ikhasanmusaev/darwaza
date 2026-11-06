#!/usr/bin/env bash
# Поднять собственный sshd для живых тестов — БЕЗ прав администратора.
#
# Живые тесты (`sftp_live`, `keyboard_interactive`) помечены `#[ignore]`,
# потому что им нужен настоящий сервер. Раньше это означало «настройте
# системный sshd»: sudo, правка /etc — и тесты не запускались месяцами.
#
# Этот sshd слушает 127.0.0.1:2222, живёт в своём каталоге и в системе не
# трогает ничего. Окупился в первый же день: нашёл дефект #208 — запись файла
# агентом не создавала файл и не обрезала его.
#
#   scripts/live-sshd.sh start   # поднять и напечатать команду для тестов
#   scripts/live-sshd.sh stop    # остановить
set -euo pipefail

dir=${LIVE_SSHD_DIR:-${TMPDIR:-/tmp}/darwaza-live-sshd}
sshd_bin=$(command -v sshd || echo /usr/sbin/sshd)

start() {
  mkdir -p "$dir" && chmod 700 "$dir"
  [ -f "$dir/host_ed25519" ]   || ssh-keygen -q -t ed25519 -f "$dir/host_ed25519" -N ""
  [ -f "$dir/client_ed25519" ] || ssh-keygen -q -t ed25519 -f "$dir/client_ed25519" -N ""
  cp "$dir/client_ed25519.pub" "$dir/authorized_keys"
  chmod 600 "$dir/authorized_keys"

  cat > "$dir/sshd_config" <<EOF
Port 2222
ListenAddress 127.0.0.1
HostKey $dir/host_ed25519
AuthorizedKeysFile $dir/authorized_keys
PasswordAuthentication no
UsePAM no
# Иначе sshd откажется работать из-за прав на каталог во временной папке.
StrictModes no
PidFile $dir/sshd.pid
Subsystem sftp internal-sftp
EOF

  "$sshd_bin" -f "$dir/sshd_config" -E "$dir/sshd.log"
  echo "sshd слушает 127.0.0.1:2222, журнал: $dir/sshd.log"
  echo
  echo "Запуск живых тестов:"
  echo "  cd src-tauri && DARWAZA_LIVE_KEY=$dir/client_ed25519 \\"
  echo "    cargo test --test sftp_live -- --ignored --nocapture"
}

stop() {
  [ -f "$dir/sshd.pid" ] || { echo "не запущен"; return 0; }
  kill "$(cat "$dir/sshd.pid")" && rm -f "$dir/sshd.pid"
  echo "остановлен"
}

case "${1:-start}" in
  start) start ;;
  stop)  stop ;;
  *) echo "использование: $0 [start|stop]" >&2; exit 2 ;;
esac
