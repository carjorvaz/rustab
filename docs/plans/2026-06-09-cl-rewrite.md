# rustab Common Lisp Rewrite — Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Port rustab from Rust to Common Lisp, preserving wire protocol, CLI interface, and behavior, while expressing the implementation in idiomatic CL.

**Architecture:** Three ASDF systems mirror the Rust crate structure:
- `rustab/protocol` — native messaging framing, RPC types, socket helpers, browser metadata, tab/window ID parsing
- `rustab/cli` — CLI entry point (clingon), all subcommands, socket discovery, output formatting
- `rustab/mediator` — bridge daemon: stdin/stdout (browser extension) ↔ Unix sockets (CLI clients), threaded

The browser extension (JS) is unchanged — it speaks native messaging protocol, which we reimplement in CL.

**Tech Stack:** SBCL, ASDF, FiveAM, com.inuoe.jzon, usocket, bordeaux-threads, clingon, cxml (plist XML parsing), Nix packaging via `sb-ext:save-lisp-and-die`.

**Source repo:** `/Users/cjv/Documents/rustab` (existing Rust repo; CL source goes in `src/` alongside `crates/`)

---

## Conventions

- Package prefix: `rustab` (main), subpackages `rustab.protocol`, `rustab.cli`, `rustab.mediator`
- Predicates: `-p` suffix (e.g., `pid-alive-p`)
- Constants: `+name+` via `alexandria:define-constant`
- Global vars: `*name*` via `defvar`/`defparameter`
- Conditions for error paths, not string returns
- FiveAM for tests, test system `rustab/test`
- `just` targets: `build`, `test`, `validate`
- Binary output: `bin/rustab` (CLI), `bin/rustab-mediator` (mediator)

---

## Phase 1: Project Scaffold + Protocol Core

### Task 1: Create ASDF system definitions

**Objective:** Wire up three ASDF systems with dependency ordering.

**Files:**
- Create: `src/protocol.lisp`
- Create: `src/cli.lisp`
- Create: `src/mediator.lisp`
- Create: `rustab.asd`
- Create: `test/test-protocol.lisp`
- Create: `test/test-mediator.lisp`
- Create: `rustab-test.asd`

**Step 1:** Create `rustab.asd`:

```lisp
;;;; rustab.asd — Browser tab management from the terminal.

(asdf:defsystem #:rustab/protocol
  :description "Native messaging framing, RPC types, socket helpers, browser metadata"
  :author "Carlos J. Vaz"
  :license "AGPL-3.0-or-later"
  :version "0.2.0"
  :depends-on (#:alexandria #:com.inuoe.jzon #:usocket #:bordeaux-threads)
  :serial t
  :components ((:module "protocol"
                :components ((:file "package")
                             (:file "framing")
                             (:file "types")
                             (:file "socket")
                             (:file "browser")
                             (:file "ids")))))

(asdf:defsystem #:rustab/cli
  :description "CLI for browser tab management"
  :version "0.2.0"
  :depends-on (#:rustab/protocol #:clingon)
  :serial t
  :components ((:module "cli"
                :components ((:file "package")
                             (:file "output")
                             (:file "input")
                             (:file "listing")
                             (:file "client")
                             (:file "doctor")
                             (:file "install")
                             (:file "synced")
                             (:file "commands")
                             (:file "main")))))

(asdf:defsystem #:rustab/mediator
  :description "Bridge daemon between browser extension and CLI clients"
  :version "0.2.0"
  :depends-on (#:rustab/protocol)
  :serial t
  :components ((:module "mediator"
                :components ((:file "package")
                             (:file "detect")
                             (:file "bridge")
                             (:file "main")))))
```

**Step 2:** Create `rustab-test.asd`:

```lisp
;;;; rustab-test.asd

(asdf:defsystem #:rustab/test
  :depends-on (#:rustab/protocol #:rustab/cli #:fiveam)
  :serial t
  :components ((:module "test"
                :components ((:file "package")
                             (:file "test-protocol")
                             (:file "test-ids")
                             (:file "test-client")))))
```

**Step 3:** Run `(asdf:load-system :rustab/test)` to verify wiring.

**Commit:** `feat: scaffold ASDF systems for protocol/cli/mediator/test`

---

### Task 2: Protocol — native messaging framing

**Objective:** Implement 4-byte LE length-prefixed JSON read/write on binary streams.

**Files:**
- Create: `src/protocol/package.lisp`
- Create: `src/protocol/framing.lisp`
- Create: `test/test-protocol.lisp` (partial)

**Step 1:** Create `src/protocol/package.lisp`:

```lisp
(defpackage #:rustab.protocol
  (:use #:cl)
  (:local-nicknames (#:jzon #:com.inuoe.jzon)
                     (#:bt #:bordeaux-threads)
                     (#:a #:alexandria))
  (:export
   ;; framing
   #:read-message
   #:read-message-lenient
   #:write-message
   ;; constants
   #:+request-timeout-secs+
   #:+orion-browser-request-timeout-secs+
   #:+native-host-name+
   #:+chrome-extension-id+
   #:+firefox-extension-id+
   #:+browsers+
   ;; types
   #:rpc-request #:make-rpc-request
   #:rpc-request-id #:rpc-request-method #:rpc-request-params
   #:rpc-response #:make-rpc-response
   #:rpc-response-id #:rpc-response-result #:rpc-response-error
   #:rpc-response-result-for
   #:tab-info #:make-tab-info
   #:tab-info-id #:tab-info-title #:tab-info-url #:tab-info-active
   #:tab-info-window-id #:tab-info-index #:tab-info-pinned
   #:window-info #:make-window-info
   #:window-info-lightweight #:make-window-info-lightweight
   ;; socket
   #:socket-dir #:prepare-socket-dir #:validate-socket-dir
   #:socket-path #:parse-socket-name
   ;; browser
   #:browser-prefix #:browser-manifest-info
   #:browser-manifest-info-name #:browser-manifest-info-config-dir
   #:browser-manifest-info-manifest-subdir #:browser-manifest-info-firefox-p
   ;; ids
   #:tab-ref #:make-tab-ref #:tab-ref-prefix #:tab-ref-mediator-pid #:tab-ref-tab-id
   #:window-ref #:make-window-ref #:window-ref-prefix #:window-ref-mediator-pid #:window-ref-window-id
   #:format-tab-id #:format-window-id
   #:parse-tab-id #:parse-window-id
   ;; timeouts
   #:browser-request-timeout #:client-request-timeout
   ;; pid
   #:pid-alive-p))
```

**Step 2:** Create `src/protocol/framing.lisp`:

```lisp
(in-package #:rustab.protocol)

(defconstant +max-inbound-message-bytes+ (* 64 1024 1024))
(defconstant +max-outbound-message-bytes+ (* 1 1024 1024))

(defun read-uint32-le (stream)
  "Read a 4-byte little-endian unsigned integer from STREAM."
  (let ((buf (make-array 4 :element-type '(unsigned-byte 8))))
    (read-sequence buf stream)
    (logior (aref buf 0)
            (ash (aref buf 1) 8)
            (ash (aref buf 2) 16)
            (ash (aref buf 3) 24))))

(defun write-uint32-le (value stream)
  "Write VALUE as a 4-byte little-endian unsigned integer to STREAM."
  (write-byte (logand value #xFF) stream)
  (write-byte (logand (ash value -8) #xFF) stream)
  (write-byte (logand (ash value -16) #xFF) stream)
  (write-byte (logand (ash value -24) #xFF) stream))

(defun read-message (stream)
  "Read a native-messaging-framed JSON message from STREAM.
   Returns a parsed JSON object (hash-table).
   Signals an error on empty messages, oversized payloads, or malformed JSON."
  (let* ((len (read-uint32-le stream)))
    (when (zerop len)
      (error "empty message"))
    (when (> len +max-inbound-message-bytes+)
      (error "message exceeds 64 MiB"))
    (let ((buf (make-array len :element-type '(unsigned-byte 8))))
      (read-sequence buf stream)
      (jzon:parse (flexi-streams:octets-to-string buf :external-format :utf-8)))))

(defun read-message-lenient (stream)
  "Read a native-messaging-framed JSON message, tolerating short length prefixes.
   Orion 1.0.x uses JS string length instead of UTF-8 byte length.
   If JSON parsing fails with EOF error, keeps reading bytes until valid."
  (let* ((len (read-uint32-le stream)))
    (when (zerop len)
      (error "empty message"))
    (when (> len +max-inbound-message-bytes+)
      (error "message exceeds 64 MiB"))
    (let ((buf (make-array len :element-type '(unsigned-byte 8))))
      (read-sequence buf stream)
      (loop
        (handler-case
            (return-from read-message-lenient
              (jzon:parse (flexi-streams:octets-to-string buf :external-format :utf-8)))
          (jzon:json-parse-error (e)
            (declare (ignore e))
            (when (>= (length buf) +max-inbound-message-bytes+)
              (error "message exceeds recovery limit"))
            (let ((extra (make-array 1 :element-type '(unsigned-byte 8))))
              (read-sequence extra stream)
              (setf buf (concatenate '(vector (unsigned-byte 8)) buf extra))))))))

(defun write-message (stream message)
  "Write a native-messaging-framed JSON message to STREAM.
   MESSAGE is a JSON-serializable object (hash-table, alist, etc.).
   Signals an error if the payload exceeds 1 MiB."
  (let* ((json-string (jzon:stringify message))
         (payload (flexi-streams:string-to-octets json-string :external-format :utf-8)))
    (when (> (length payload) +max-outbound-message-bytes+)
      (error "message exceeds 1 MiB outbound limit"))
    (write-uint32-le (length payload) stream)
    (write-sequence payload stream)
    (force-output stream)))
```

**Step 3:** Create initial `test/test-protocol.lisp` with framing round-trip tests.

**Step 4:** Run tests, verify pass.

**Commit:** `feat: protocol framing — native messaging read/write with lenient Orion recovery`

---

### Task 3: Protocol — RPC types

**Objective:** Define RPC request/response structs and tab/window info types.

**Files:**
- Create: `src/protocol/types.lisp`

**Step 1:** Create `src/protocol/types.lisp`:

```lisp
(in-package #:rustab.protocol)

;;; RPC request envelope.

(defstruct (rpc-request (:constructor make-rpc-request (id method &optional (params (make-hash-table)))))
  (id 1 :type (unsigned-byte 64))
  (method "" :type string)
  (params nil))

(defun rpc-request-default (method &optional (params (make-hash-table)))
  "Construct a request with the default CLI request ID (1)."
  (make-rpc-request 1 method params))

;;; RPC response envelope.

(defstruct (rpc-response (:constructor make-rpc-response (id &key result error)))
  (id 0 :type (unsigned-byte 64))
  (result nil)
  (error nil))

(defun rpc-response-result-for (response request-id)
  "Extract the result from RESPONSE, validating it matches REQUEST-ID.
   Returns the result value, or signals an error."
  (let ((rid (rpc-response-id response)))
    (unless (= rid request-id)
      (error "response id ~D did not match request id ~D" rid request-id)))
  (if (rpc-response-error response)
      (error "~A" (rpc-response-error response))
      (or (rpc-response-result response)
          (error "invalid response"))))

;;; Tab info from browser extension.

(defstruct tab-info
  (id 0 :type (unsigned-byte 64))
  (title "" :type string)
  (url "" :type string)
  (active nil :type boolean)
  (window-id 0 :type (unsigned-byte 64))
  (index 0 :type (signed-byte 64))
  (pinned nil :type boolean))

;;; Window info from browser extension.

(defstruct window-info
  (id 0 :type (unsigned-byte 64))
  (focused nil :type boolean)
  (window-type "" :type string)
  (state "" :type string)
  (incognito nil :type boolean)
  (tab-count 0 :type (unsigned-byte 64))
  (active-tab-id nil :type (or null (unsigned-byte 64)))
  (active-tab-title "" :type string)
  (active-tab-url "" :type string))

;;; Lightweight window info (no tab population — fast for 900+ tabs).

(defstruct window-info-lightweight
  (id 0 :type (unsigned-byte 64))
  (focused nil :type boolean)
  (window-type "" :type string)
  (state "" :type string)
  (incognito nil :type boolean))
```

**Step 2:** Add JSON deserialization helpers for converting jzon output to structs.

**Step 3:** Tests for RPC response result extraction.

**Commit:** `feat: protocol types — RPC request/response, tab-info, window-info structs`

---

### Task 4: Protocol — socket management

**Objective:** Socket directory creation/validation, path construction, name parsing.

**Files:**
- Create: `src/protocol/socket.lisp`

**Step 1:** Implement `socket-dir` (`/tmp/rustab-{uid}/`), `prepare-socket-dir` (create + chmod 700), `validate-socket-dir` (check ownership + permissions), `socket-path`, `parse-socket-name`.

Key CL translation:
- `effective-uid` → `(sb-posix:geteuid)` on SBCL
- `kill(pid, 0)` → `(sb-posix:kill pid 0)` with `sb-posix:esrch`/`sb-posix:eperm` handling
- Permission checking → `(sb-posix:stat-mode ...)` and `(logand mode #o777)`

**Step 2:** Tests for socket name parsing, directory validation.

**Commit:** `feat: protocol socket — directory management, PID liveness checking`

---

### Task 5: Protocol — browser metadata + ID parsing

**Objective:** Browser prefix mapping, manifest info table, tab/window ID parsing.

**Files:**
- Create: `src/protocol/browser.lisp`
- Create: `src/protocol/ids.lisp`

**Step 1:** `browser.lisp` — `browser-prefix` function, `+browsers+` constant (platform-conditional via `#+(or)`), `+native-host-name+`, `+chrome-extension-id+`, `+firefox-extension-id+`.

**Step 2:** `ids.lisp` — `parse-tab-id` (accepts `prefix.pid.id` and legacy `prefix.id`), `parse-window-id` (accepts `prefix.pid.w.id` and legacy `prefix.w.id`), `format-tab-id`, `format-window-id`.

**Step 3:** Comprehensive ID parsing tests (port the Rust test cases directly).

**Commit:** `feat: protocol browser+ids — prefixes, manifest metadata, tab/window ID parsing`

---

## Phase 2: CLI Layer

### Task 6: CLI — output + input helpers

**Objective:** TSV/JSON output formatting, stdin tab ID collection.

**Files:**
- Create: `src/cli/package.lisp`
- Create: `src/cli/output.lisp`
- Create: `src/cli/input.lisp`

**Step 1:** `package.lisp` — `rustab.cli` package using `rustab.protocol`.

**Step 2:** `output.lisp` — `print-json` (pretty-print via jzon), `print-tsv` (tab-delimited rows).

**Step 3:** `input.lisp` — `collect-tab-ids` (from args or stdin pipe, tab-delimited first field), `parse-tab-ids`, `parse-window-arg`, `validate-move-index`, `validate-open-index`.

Key CL: `(listen *standard-input*)` and `(read-line *standard-input* nil)` for pipe detection. Use `sb-posix:isatty` or `(interactive-stream-p *standard-input*)` for terminal detection.

**Commit:** `feat: cli output+input — JSON/TSV formatting, stdin tab ID collection`

---

### Task 7: CLI — socket discovery + RPC client

**Objective:** Discover connected browser sockets, send RPC requests, resolve socket by tab/window ID.

**Files:**
- Create: `src/cli/client.lisp`

**Step 1:** `discover-sockets` — scan socket dir, filter by browser name, check PID liveness.

**Step 2:** `send-rpc` — connect to Unix socket, write RPC request, read response with timeout.

Key CL: `usocket:socket-connect` for Unix sockets (or `sb-bsd-sockets:make-unix-socket` directly if usocket doesn't support Unix domain). Timeout via `bt:with-timeout` or a simple `select`-based approach.

**Step 3:** `resolve-socket`, `resolve-socket-for-tab-ref`, `resolve-socket-for-window-ref`, `socket-for-raw-window-id`.

**Step 4:** Port the Rust client tests.

**Commit:** `feat: cli client — socket discovery, RPC calls, socket resolution`

---

### Task 8: CLI — list + windows commands

**Objective:** `rustab list` and `rustab windows` subcommands.

**Files:**
- Create: `src/cli/listing.lisp`
- Create: `src/cli/commands.lisp` (partial)

**Step 1:** `listing.lisp` — `fetch-tab-listings` (fan out to all sockets, collect), `sort-tab-listings`, `fetch-window-listings`, `format-tab-id-full` (with browser prefix + PID).

**Step 2:** Wire up `list` and `windows` subcommands in `commands.lisp`.

**Step 3:** Test with live browsers if available.

**Commit:** `feat: cli list+windows — tab and window listing with TSV/JSON output`

---

### Task 9: CLI — close, move, activate, open commands

**Objective:** Remaining tab manipulation subcommands.

**Files:**
- Extend: `src/cli/commands.lisp`

**Step 1:** `cmd-close` — parse tab IDs, fan out to sockets, send `close_tabs` RPC.

**Step 2:** `cmd-move` — parse target window/tab, tab IDs, send `move_tabs` RPC.

**Step 3:** `cmd-activate` — parse tab ID, send `activate_tab` RPC.

**Step 4:** `cmd-open` — send `open_tab` RPC with URL, optional browser/window/index.

**Step 5:** `cmd-clients` — list connected browser sockets.

**Commit:** `feat: cli close+move+activate+open — tab manipulation commands`

---

### Task 10: CLI — doctor command

**Objective:** Diagnostic checks for socket dir, manifests, connected browsers.

**Files:**
- Create: `src/cli/doctor.lisp`

**Step 1:** Implement `cmd-doctor` with report struct (ok/warn/error counts).

**Step 2:** Checks: socket directory exists and is private, native manifests present and valid, connected browsers respond to RPC.

**Step 3:** Platform-specific checks (Orion extension manifest on macOS).

**Commit:** `feat: cli doctor — diagnostic checks for socket dir, manifests, connectivity`

---

### Task 11: CLI — install command

**Objective:** Install native messaging manifests for detected browsers.

**Files:**
- Create: `src/cli/install.lisp`

**Step 1:** `cmd-install` — detect browsers, write Chromium/Firefox manifest JSON files.

**Step 2:** `manifest-target-dirs` — platform-specific manifest directory resolution.

**Commit:** `feat: cli install — native messaging manifest installation`

---

### Task 12: CLI — synced tabs (macOS plist parsing)

**Objective:** `rustab synced list` — read Orion synced tabs from plist files.

**Files:**
- Create: `src/cli/synced.lisp`

**Step 1:** Plist XML parsing using `cxml` (SAX handler for plist format).

**Step 2:** `parse-orion-synced-snapshot` — parse `.local_named_windows.plist`.

**Step 3:** `parse-orion-current-session-state` — parse `browser_session_state.plist` (nested JSON in plist strings).

**Step 4:** `latest-non-empty-orion-snapshot` — find newest `bk_*/.local_named_windows.plist`.

**Step 5:** Port the Rust plist fixtures as FiveAM test inputs.

**Commit:** `feat: cli synced — Orion synced tabs from plist parsing`

---

### Task 13: CLI — main entry point + clingon wiring

**Objective:** Wire all commands into a single CLI binary.

**Files:**
- Create: `src/cli/main.lisp`

**Step 1:** Define clingon command tree with all subcommands.

**Step 2:** Signal handler for SIGPIPE reset (`(sb-unix:unix-signal sb-unix:sigpipe :default)`).

**Step 3:** Entry point: `(defun main () ...)` that parses args and dispatches.

**Step 4:** Build script: `(sb-ext:save-lisp-and-die "bin/rustab" :toplevel #'rustab.cli:main :executable t)`.

**Commit:** `feat: cli main — clingon wiring, SIGPIPE handling, binary delivery`

---

## Phase 3: Mediator Layer

### Task 14: Mediator — browser detection

**Objective:** Detect which browser launched the mediator from CLI args and parent process.

**Files:**
- Create: `src/mediator/package.lisp`
- Create: `src/mediator/detect.lisp`

**Step 1:** `detect-browser` — inspect `sb-ext:*posix-argv*` for extension URLs, inspect parent process name via `/proc/self/status` (Linux) or `ps -o comm= -p <ppid>` (macOS).

**Step 2:** Port the Firefox/Chromium launch hint tables.

**Step 3:** Tests for browser detection logic.

**Commit:** `feat: mediator detect — browser identification from launch context`

---

### Task 15: Mediator — bidirectional bridge

**Objective:** The core mediator: stdin/stdout (browser) ↔ Unix sockets (CLI clients).

**Files:**
- Create: `src/mediator/bridge.lisp`

**Step 1:** Implement the threaded bridge:

```lisp
;;; Shared state.
(defvar *pending-responses* nil)      ; hash-table: internal-id → channel
(defvar *pending-lock* nil)           ; bt:lock
(defvar *next-request-id* 0)          ; atomic counter
(defvar *browser-tx* nil)             ; mailbox for messages to browser

;;; Threads:
;;; 1. stdin-reader — reads from browser extension, routes responses to pending
;;; 2. stdout-writer — reads from browser-tx mailbox, writes to browser
;;; 3. socket-accept — accepts CLI client connections, spawns handler thread
;;; 4. client-handler — per-client: reads request, assigns unique ID, forwards
;;;    to browser, waits for response with timeout, writes back to client
```

Key CL translation:
- `tokio::mpsc` → `sb-concurrency:mailbox` or simple queue + condition variable
- `tokio::oneshot` → `bt:condition-wait` with a per-request condition variable
- `Arc<Mutex<HashMap>>` → hash-table + `bt:with-lock-held`
- `tokio::time::timeout` → `bt:with-timeout`
- `tokio::select!` → no direct equivalent; each thread runs independently, shutdown via `*shutdown*` flag

**Step 2:** Graceful shutdown: when stdin closes (browser exits), set shutdown flag, close listener, join threads.

**Step 3:** Stale socket cleanup on startup.

**Commit:** `feat: mediator bridge — threaded stdin/stdout ↔ Unix socket multiplexing`

---

### Task 16: Mediator — main entry point

**Objective:** Wire mediator into a deliverable binary.

**Files:**
- Create: `src/mediator/main.lisp`

**Step 1:** `main` function: detect browser, prepare socket dir, cleanup stale sockets, bind listener, spawn bridge threads, wait for shutdown.

**Step 2:** Build script: `(sb-ext:save-lisp-and-die "bin/rustab-mediator" :toplevel #'rustab.mediator:main :executable t)`.

**Commit:** `feat: mediator main — binary delivery for rustab-mediator`

---

## Phase 4: Build, Test, Package

### Task 17: justfile + validation

**Objective:** `just build`, `just test`, `just validate` targets.

**Files:**
- Create: `justfile`

**Step 1:**
```just
build:
    sbcl --noinform --non-interactive --eval '(asdf:load-system :rustab/cli)' --eval '(asdf:load-system :rustab/mediator)' --eval '(sb-ext:save-lisp-and-die "bin/rustab" :toplevel (lambda () (rustab.cli:main)) :executable t)' --eval '(sb-ext:save-lisp-and-die "bin/rustab-mediator" :toplevel (lambda () (rustab.mediator:main)) :executable t)'

test:
    sbcl --noinform --non-interactive --eval '(asdf:test-system :rustab/test)'

validate: test
    @echo "All checks passed."
```

**Commit:** `chore: justfile with build/test/validate targets`

---

### Task 18: Nix packaging

**Objective:** `nix build` produces `bin/rustab` and `bin/rustab-mediator`.

**Files:**
- Create: `flake.nix` (or extend existing if present)

**Step 1:** SBCL binary build via `sbcl.withPackages` or `buildASDFSystem`.

**Step 2:** Verify `nix build` produces working binaries.

**Commit:** `feat: nix packaging for CL rustab binaries`

---

### Task 19: Integration verification

**Objective:** Verify the CL build behaves identically to the Rust build.

**Step 1:** Build both Rust and CL versions.

**Step 2:** Compare output of `rustab list --format json` and `rustab clients` against running browsers.

**Step 3:** Test pipe workflow: `rustab list | rustab close`.

**Step 4:** Test `rustab doctor` output.

**Commit:** `test: integration verification — CL vs Rust behavioral parity`

---

## Phase 5: Migration

### Task 20: Replace Rust binary paths

**Objective:** Switch `~/.local/bin/rustab` and `~/.local/state/rustab/` to CL binaries.

**Step 1:** Update symlink or copy CL binary to `~/.local/bin/rustab`.

**Step 2:** Update mediator path in native messaging manifests (re-run `rustab install`).

**Step 3:** Verify end-to-end: extension → mediator → CLI → extension.

**Commit:** `feat: switch to CL rustab binaries`

---

## Key Design Decisions

1. **Threads, not async.** The mediator uses `bordeaux-threads` with locks and condition variables. The CLI is purely synchronous. No green threads, no iolib event loops.

2. **`com.inuoe.jzon` for JSON.** Fast, no external deps, handles the wire format. Use `jzon:parse` for deserialization and `jzon:stringify` for serialization.

3. **`usocket` for Unix sockets.** If `usocket` doesn't support Unix domain sockets well, fall back to `sb-bsd-sockets:make-unix-socket` directly.

4. **`flexi-streams` for UTF-8.** Needed for the framing layer (byte vector ↔ string conversion).

5. **`cxml` for plist parsing.** macOS-only synced tab feature needs XML plist parsing. `cxml` gives us a SAX handler we can build a focused plist parser on.

6. **Conditions/restarts for errors.** `rpc-response-result-for` signals `error` (or a custom condition) instead of returning `Result`. Callers use `handler-case`/`handler-bind`.

7. **Platform conditionals.** Use `#+darwin` / `#+linux` for platform-specific code (PID checking, plist parsing, browser paths).

8. **Binary delivery.** Two separate binaries via `sb-ext:save-lisp-and-die` — `bin/rustab` (CLI) and `bin/rustab-mediator` (mediator daemon).

---

## What We're NOT Changing

- The browser extension (JS) — untouched
- The wire protocol — byte-compatible
- The CLI interface — same subcommands, same flags, same output formats
- The socket directory layout — `/tmp/rustab-{uid}/{browser}-{pid}.sock`
- The tab/window ID formats — `prefix.pid.id`, `prefix.pid.w.id`
