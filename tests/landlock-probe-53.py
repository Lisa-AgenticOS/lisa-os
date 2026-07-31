# Does a Landlock ruleset shaped like ours actually stop a write outside
# the project? Asked of the kernel, not of the documentation.
import ctypes, os, struct, sys, tempfile

NR_CREATE, NR_ADD, NR_RESTRICT = 444, 445, 446
libc = ctypes.CDLL("libc.so.6", use_errno=True)

# ABI v1 filesystem access rights.
FS_EXECUTE, FS_WRITE_FILE, FS_READ_FILE, FS_READ_DIR = 1, 2, 4, 8
FS_REMOVE_DIR, FS_REMOVE_FILE = 16, 32
FS_MAKE_CHAR, FS_MAKE_DIR, FS_MAKE_REG, FS_MAKE_SOCK = 64, 128, 256, 512
FS_MAKE_FIFO, FS_MAKE_BLOCK, FS_MAKE_SYM = 1024, 2048, 4096
ALL_V1 = (1 << 13) - 1
READ_V1 = FS_EXECUTE | FS_READ_FILE | FS_READ_DIR

class RulesetAttr(ctypes.Structure):
    _fields_ = [("handled_access_fs", ctypes.c_uint64)]

class PathBeneathAttr(ctypes.Structure):
    # The kernel's landlock_path_beneath_attr is __attribute__((packed)):
    # u64 then s32, twelve bytes, no tail padding. Getting this wrong
    # makes every add_rule fail with EINVAL, which looks exactly like a
    # ruleset that refuses everything.
    _pack_ = 1
    _layout_ = "ms"
    _fields_ = [("allowed_access", ctypes.c_uint64), ("parent_fd", ctypes.c_int32)]

project = tempfile.mkdtemp(prefix="proj-")
outside = tempfile.mkdtemp(prefix="outside-")

attr = RulesetAttr(ALL_V1)
rs = libc.syscall(NR_CREATE, ctypes.byref(attr), ctypes.sizeof(attr), 0)
if rs < 0:
    print("FAIL: create_ruleset:", os.strerror(ctypes.get_errno())); sys.exit(1)

def allow(path, access):
    fd = os.open(path, os.O_PATH | os.O_CLOEXEC)
    pb = PathBeneathAttr(access, fd)
    # landlock_add_rule(fd, rule_type, rule_attr, flags) — FOUR
    # arguments. Passing the struct size as `flags` is EINVAL, and looks
    # identical to a ruleset that refuses everything.
    r = libc.syscall(NR_ADD, rs, 1, ctypes.byref(pb), 0)
    err = ctypes.get_errno()
    os.close(fd)
    if r != 0:
        print(f"FAIL: add_rule({path}) -> {os.strerror(err)} "
              f"(struct size {ctypes.sizeof(pb)}, expected 12)")
        sys.exit(1)
    return r

allow(project, ALL_V1)
for d in ("/usr", "/etc", "/proc", "/dev", "/lib", "/lib64"):
    if os.path.exists(d):
        allow(d, READ_V1)

# no_new_privs, then enforce.
if libc.prctl(38, 1, 0, 0, 0) != 0:
    print("FAIL: prctl(NO_NEW_PRIVS)"); sys.exit(1)
if libc.syscall(NR_RESTRICT, rs, 0) != 0:
    print("FAIL: restrict_self:", os.strerror(ctypes.get_errno())); sys.exit(1)

print("ruleset enforced")

ok = True
try:
    with open(os.path.join(project, "inside.txt"), "w") as f:
        f.write("allowed")
    print("  write INSIDE the project: allowed  (correct)")
except OSError as e:
    print(f"  write INSIDE the project: REFUSED {e}  (WRONG — the build could not work)"); ok = False

try:
    with open(os.path.join(outside, "escaped.txt"), "w") as f:
        f.write("should never happen")
    print("  write OUTSIDE the project: allowed  (WRONG — this is the hole)"); ok = False
except OSError as e:
    print(f"  write OUTSIDE the project: refused ({e.strerror})  (correct)")

try:
    home_file = os.path.expanduser("~/.landlock-escape-probe")
    with open(home_file, "w") as f:
        f.write("x")
    print("  write to $HOME: allowed  (WRONG)"); ok = False
    os.unlink(home_file)
except OSError as e:
    print(f"  write to $HOME: refused ({e.strerror})  (correct)")

try:
    open("/usr/bin/env", "rb").read(4)
    print("  read the toolchain: allowed  (correct)")
except OSError as e:
    print(f"  read the toolchain: REFUSED {e}  (WRONG)"); ok = False

print("PASS" if ok else "FAIL")
sys.exit(0 if ok else 1)
