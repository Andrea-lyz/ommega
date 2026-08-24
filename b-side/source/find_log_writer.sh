#!/system/bin/sh
# 找出所有打开 relay.log 的进程
for f in /proc/[0-9]*/fd/*; do
  target=$(readlink "$f" 2>/dev/null)
  case "$target" in
    *relay.log*)
      pid=$(echo "$f" | cut -d/ -f3)
      comm=$(cat /proc/$pid/comm 2>/dev/null)
      cmd=$(cat /proc/$pid/cmdline 2>/dev/null | tr '\0' ' ')
      pos=$(awk '/^pos:/{print $2}' /proc/$pid/fdinfo/$(basename $f) 2>/dev/null)
      echo "PID=$pid comm=$comm fd=$(basename $f) pos=$pos"
      echo "  cmdline: $cmd"
      ;;
  esac
done
echo "--- 所有 relay/daemon 进程 ---"
ps -A -o PID,PPID,ETIME,NAME | grep -iE "relay|daemon|sh"
