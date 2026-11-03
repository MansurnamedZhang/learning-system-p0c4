// Linux-only candidate. Build each role separately with the reviewed -X inputs.
package main

import (
	"crypto/sha256"
	"encoding/hex"
	"errors"
	"fmt"
	"io"
	"os"
	"strconv"
	"strings"
	"syscall"
)

const (
	transportUser = "kw-b1-transport"
	gatePath      = "/usr/local/libexec/knowweave-c4-b1-ssh-gate"
	launcherPath  = "/usr/local/libexec/knowweave-c4-b1-receiver-launcher"
	receiverPath  = "/usr/local/libexec/knowweave-c4-b1-receiver"
	lockPath      = "/etc/knowweave-c4-b1/receiver-launch.lock"
	forcedCommand = "knowweave-c4-b1-fixed"
	maxReceiver   = 64 * 1024 * 1024
	refused       = 125
)

var role, transportUID, transportGID, receiverSHA256 string

func numericID(text string) (int, bool) {
	id, err := strconv.Atoi(text)
	return id, err == nil && id >= 1000 && id <= 60000 && strconv.Itoa(id) == text
}

func exactRequest(args []string, original string) bool {
	return len(args) == 3 && args[1] == "-c" && args[2] == forcedCommand && original == receiverPath
}

func pipeEnds(files []*os.File) bool {
	if len(files) != 3 {
		return false
	}
	type identity struct{ device, inode uint64 }
	seen := make(map[identity]bool, 3)
	for index, file := range files {
		var stat syscall.Stat_t
		flags, _, err := syscall.Syscall(syscall.SYS_FCNTL, file.Fd(), syscall.F_GETFL, 0)
		want := uintptr(syscall.O_WRONLY)
		if index == 0 {
			want = syscall.O_RDONLY
		}
		if err != 0 || flags&syscall.O_ACCMODE != want || syscall.Fstat(int(file.Fd()), &stat) != nil ||
			stat.Mode&syscall.S_IFMT != syscall.S_IFIFO {
			return false
		}
		key := identity{stat.Dev, stat.Ino}
		if seen[key] {
			return false
		}
		seen[key] = true
	}
	return true
}

func inheritableFDsClosed() bool {
	entries, err := os.ReadDir("/proc/self/fd")
	if err != nil || len(entries) > 64 {
		return false
	}
	for _, entry := range entries {
		fd, err := strconv.Atoi(entry.Name())
		if err != nil || fd < 0 {
			return false
		}
		if fd <= 2 {
			continue
		}
		flags, _, errno := syscall.Syscall(syscall.SYS_FCNTL, uintptr(fd), syscall.F_GETFD, 0)
		// ReadDir has already closed its own enumeration descriptor.
		if errno != syscall.EBADF && (errno != 0 || flags&syscall.FD_CLOEXEC == 0) {
			return false
		}
	}
	return true
}

func gate(uid, gid int, files []*os.File) int {
	if os.Getuid() != uid || os.Geteuid() != uid || os.Getgid() != gid || os.Getegid() != gid ||
		!exactRequest(os.Args, os.Getenv("SSH_ORIGINAL_COMMAND")) || !pipeEnds(files) || !inheritableFDsClosed() {
		return refused
	}
	// No token from the request reaches argv or the sudo environment.
	_ = syscall.Exec("/usr/bin/sudo", []string{"/usr/bin/sudo", "-n", "-u", "root", "-g", "root", "--", launcherPath},
		[]string{"PATH=/usr/sbin:/usr/bin:/bin", "LANG=C", "LC_ALL=C"})
	return refused
}

func rootProcess(uid, gid string) bool {
	if len(os.Args) != 1 || os.Getenv("SUDO_USER") != transportUser || os.Getenv("SUDO_UID") != uid ||
		os.Getenv("SUDO_GID") != gid {
		return false
	}
	groups, err := os.Getgroups()
	if err != nil || (len(groups) != 0 && (len(groups) != 1 || groups[0] != 0)) {
		return false
	}
	raw, err := os.ReadFile("/proc/self/status")
	if err != nil || len(raw) > 8192 {
		return false
	}
	want := map[string]string{"Uid:": "0 0 0 0", "Gid:": "0 0 0 0", "TracerPid:": "0"}
	for _, line := range strings.Split(string(raw), "\n") {
		fields := strings.Fields(line)
		if len(fields) == 0 {
			continue
		}
		if value, ok := want[fields[0]]; ok {
			if strings.Join(fields[1:], " ") != value {
				return false
			}
			delete(want, fields[0])
		}
	}
	return len(want) == 0
}

func protectedFile(path string, mode uint32) (*os.File, syscall.Stat_t, error) {
	var current syscall.Stat_t
	flags := syscall.O_RDONLY | syscall.O_CLOEXEC | syscall.O_NOFOLLOW
	dir, err := syscall.Open("/", flags|syscall.O_DIRECTORY, 0)
	if err != nil {
		return nil, current, err
	}
	defer func() { _ = syscall.Close(dir) }()
	parts := strings.Split(strings.TrimPrefix(path, "/"), "/")
	for _, part := range append([]string{"."}, parts[:len(parts)-1]...) {
		next, err := syscall.Openat(dir, part, flags|syscall.O_DIRECTORY, 0)
		if err != nil {
			return nil, current, err
		}
		_ = syscall.Close(dir)
		dir = next
		if syscall.Fstat(dir, &current) != nil || current.Uid != 0 || current.Gid != 0 || current.Mode&0022 != 0 {
			return nil, current, errors.New("untrusted ancestor")
		}
	}
	fd, err := syscall.Openat(dir, parts[len(parts)-1], flags|syscall.O_NONBLOCK, 0)
	if err != nil {
		return nil, current, err
	}
	if syscall.Fstat(fd, &current) != nil || current.Uid != 0 || current.Gid != 0 ||
		current.Mode != syscall.S_IFREG|mode || current.Nlink != 1 {
		_ = syscall.Close(fd)
		return nil, current, errors.New("untrusted file")
	}
	return os.NewFile(uintptr(fd), path), current, nil
}

func receiverMatches(file *os.File, before syscall.Stat_t) bool {
	if before.Size < 64 || before.Size > maxReceiver || len(receiverSHA256) != 64 {
		return false
	}
	hash := sha256.New()
	count, err := io.Copy(hash, io.LimitReader(file, maxReceiver+1))
	var after, named syscall.Stat_t
	return err == nil && count == before.Size && hex.EncodeToString(hash.Sum(nil)) == receiverSHA256 &&
		syscall.Fstat(int(file.Fd()), &after) == nil && syscall.Lstat(receiverPath, &named) == nil &&
		sameStamp(before, after) && sameStamp(before, named)
}

func sameStamp(a, b syscall.Stat_t) bool {
	return a.Dev == b.Dev && a.Ino == b.Ino && a.Mode == b.Mode && a.Nlink == b.Nlink &&
		a.Uid == b.Uid && a.Gid == b.Gid && a.Size == b.Size && a.Mtim == b.Mtim && a.Ctim == b.Ctim
}

func waitReceiver(process *os.Process, files []*os.File) int {
	// Leaving a write copy in the parent can prevent the original S EOF gate.
	closed := true
	for _, file := range files {
		if file.Close() != nil {
			closed = false
		}
	}
	state, err := process.Wait()
	if err != nil || !closed {
		return refused
	}
	if result, ok := state.Sys().(syscall.WaitStatus); ok {
		if result.Exited() {
			return result.ExitStatus()
		}
		if result.Signaled() {
			return 128 + int(result.Signal())
		}
	}
	return refused
}

func launcher(files []*os.File) int {
	if !rootProcess(transportUID, transportGID) || !pipeEnds(files) || !inheritableFDsClosed() {
		return refused
	}
	lock, stamp, err := protectedFile(lockPath, 0600)
	if err != nil {
		return refused
	}
	defer lock.Close()
	if stamp.Size != 0 || syscall.Flock(int(lock.Fd()), syscall.LOCK_EX|syscall.LOCK_NB) != nil {
		return refused
	}
	receiver, stamp, err := protectedFile(receiverPath, 0500)
	if err != nil {
		return refused
	}
	defer receiver.Close()
	if !receiverMatches(receiver, stamp) {
		return refused
	}
	// Only root can change a trusted ancestor or receiver. C-install excludes
	// concurrent root writers for the operation. The receiver reopens and
	// authenticates its actual ELF and this still-live native parent.
	process, err := os.StartProcess(receiverPath, []string{receiverPath}, &os.ProcAttr{
		Dir: "/", Env: []string{}, Files: files,
	})
	if err != nil {
		return refused
	}
	return waitReceiver(process, files)
}

func main() {
	uid, uidOK := numericID(transportUID)
	gid, gidOK := numericID(transportGID)
	files := []*os.File{os.Stdin, os.Stdout, os.Stderr}
	self, err := os.Readlink("/proc/self/exe")
	if err == nil && uidOK && gidOK {
		if role == "gate" && self == gatePath && receiverSHA256 == "" {
			os.Exit(gate(uid, gid, files))
		}
		if role == "launcher" && self == launcherPath {
			os.Exit(launcher(files))
		}
	}
	fmt.Fprintln(os.Stderr, "H4 fixed entry refused")
	os.Exit(refused)
}
