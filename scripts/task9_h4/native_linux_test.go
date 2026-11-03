package main

import (
	"io"
	"os"
	"testing"
	"time"
)

func TestExactRequest(t *testing.T) {
	args := []string{"login-shell", "-c", forcedCommand}
	if !exactRequest(args, receiverPath) {
		t.Fatal("fixed SSH request rejected")
	}
	for _, original := range []string{"", "internal-sftp", receiverPath + " arg", receiverPath + "\n", "sh"} {
		if exactRequest(args, original) {
			t.Fatal("accepted a non-exact original request")
		}
	}
	if exactRequest(append(args, "extra"), receiverPath) || exactRequest([]string{"-c", receiverPath}, receiverPath) {
		t.Fatal("accepted wrong shell invocation")
	}
}

func testPipe(t *testing.T) (*os.File, *os.File) {
	t.Helper()
	r, w, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = r.Close(); _ = w.Close() })
	return r, w
}

func TestPipeShape(t *testing.T) {
	input, wrongInput := testPipe(t)
	_, output := testPipe(t)
	_, stderr := testPipe(t)
	if !pipeEnds([]*os.File{input, output, stderr}) {
		t.Fatal("three distinct pipe ends rejected")
	}
	for _, bad := range [][]*os.File{{wrongInput, output, stderr}, {input, output, output}, {input, wrongInput, stderr}} {
		if pipeEnds(bad) {
			t.Fatal("accepted bad direction or duplicate inode")
		}
	}
	file, err := os.Open("/dev/null")
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()
	if pipeEnds([]*os.File{file, output, stderr}) {
		t.Fatal("accepted non-pipe input")
	}
}

func TestWaitReleasesParentWritesBeforeChildExit(t *testing.T) {
	in, control := testPipe(t)
	read, out := testPipe(t)
	_, stderr := testPipe(t)
	files := []*os.File{in, out, stderr}
	child, err := os.StartProcess("/bin/sh", []string{"/bin/sh", "-c", "printf ok; exec 1>&-; read line; exit 17"},
		&os.ProcAttr{Env: []string{}, Files: files})
	if err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() { _ = child.Kill() })
	result := make(chan int, 1)
	go func() { result <- waitReceiver(child, files) }()
	readDone := make(chan string, 1)
	go func() { raw, _ := io.ReadAll(read); readDone <- string(raw) }()
	select {
	case raw := <-readDone:
		if raw != "ok" {
			t.Fatalf("unexpected raw output %q", raw)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("output EOF blocked by a parent-held pipe")
	}
	if _, err := control.WriteString("continue\n"); err != nil {
		t.Fatal(err)
	}
	select {
	case exit := <-result:
		if exit != 17 {
			t.Fatalf("lost child exit code: %d", exit)
		}
	case <-time.After(3 * time.Second):
		t.Fatal("child not reaped")
	}
}

func TestWaitPreservesSignalAsFailure(t *testing.T) {
	in, _ := testPipe(t)
	_, out := testPipe(t)
	_, stderr := testPipe(t)
	files := []*os.File{in, out, stderr}
	child, err := os.StartProcess("/bin/sh", []string{"/bin/sh", "-c", "kill -TERM $$"},
		&os.ProcAttr{Env: []string{}, Files: files})
	if err != nil {
		t.Fatal(err)
	}
	if status := waitReceiver(child, files); status != 143 {
		t.Fatalf("signal not propagated as failure: %d", status)
	}
}
