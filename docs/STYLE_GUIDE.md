# Code Style Guide

These conventions describe general Rust design principles first and then show
how they apply to `kubefs`. They are defaults for code being touched, not a
reason to mix unrelated refactors into a behavioural change.

## Organize code around responsibilities

Give each module one coherent responsibility and keep dependencies pointing
toward stable domain concepts. Boundary adapters may depend on domain types,
but the domain model should not depend on a particular transport, runtime, or
framework.

**kubefs status: applied.** `model` owns cluster identities, `cluster` owns
remote Kubernetes access, and `fuse` owns filesystem adaptation, inode state,
and open-handle state. The model has no `fuser`, Tokio, or `kube`
dependency. The crate root re-exports `KubeFs` while keeping the `fuse` module
private; `fuse/mod.rs` is a small internal facade, and the adapter
implementation lives in `fuse/filesystem.rs`. Within a substantial `impl`,
methods are grouped by responsibility: attribute resolution, directory
handles, and regular-file handles.

## Keep the public surface narrow

Expose only the types and operations that callers need. Keep implementation
modules private when a crate-root re-export provides a clearer API, and use the
narrowest visibility that permits the intended collaboration between modules.

**kubefs status: applied.** `KubeFs` is available as `kubefs::KubeFs`, but its
implementation module, cluster adapter, domain model, registries, and handle
tables are not public API. Internal types use `pub(crate)` or `pub(super)` only
where plain module privacy is insufficient.

```rust
mod fuse;

pub use fuse::KubeFs;
```

## Use types that express domain meaning

Prefer a meaningful type over a primitive value when the value has identity,
validation rules, or a restricted set of operations. Keep raw integers and
strings at representation boundaries instead of allowing them to spread
through the program.

**kubefs status: applied.** API groups, versions, resources, namespaces, and
objects use distinct model types. Registry APIs use `INodeNo`, while raw
`u64` values are confined to inode allocation state.

```rust
fn node_for_inode(
    &self,
    inode: INodeNo,
) -> Result<Option<Node>, InodeRegistryError> {
    // ...
}
```

## Convert representations at boundaries

Store data in the representation native to its domain and convert only when
crossing into another system. This avoids impossible states and repeated
conversion failures in internal code.

**kubefs status: applied.** Kubernetes identifiers remain UTF-8 `String`
values in the model. They become `OsStr` or `OsString` only where the
filesystem API requires operating-system names.

```rust
let child_name: OsString = child.name().into();
```

Incoming filesystem names are converted in the opposite direction at the
adapter boundary before being passed to `ClusterReader`.

## Protect invariants with narrow visibility

Keep fields private when construction or mutation must preserve an invariant.
Expose constructors and the smallest useful set of accessors instead of
making representation details public for convenience.

**kubefs status: applied.** Identity fields, inode registry state, and
open-file snapshots are private. `OpenFile` exposes immutable `inode()` and
`data()` accessors, while its table owns handle allocation and mutation.

```rust
impl OpenFile {
    pub(super) fn inode(&self) -> INodeNo {
        self.inode
    }

    pub(super) fn data(&self) -> &[u8] {
        &self.data
    }
}
```

## Name operations after their outcome

Choose names that state what an operation returns or establishes. Avoid vague
verbs such as `process`, `populate`, or `generate`, and avoid
abbreviations when the longer name materially improves understanding.

**kubefs status: applied.** Names such as `file_attr`, `file_snapshot`,
`directory_entries`, `node_for_inode`, and `get_or_create_inode`
identify their outcomes. Use `inode`, `file_handle`, and `object_id` for
internal names; retain abbreviations only where an external API fixes the
field name, such as `FileAttr::ino` and `FileAttr::nlink`.

For an intentionally unused trait parameter, use `_` when its name adds no
local information. Use an underscore-prefixed name when retaining the
parameter's meaning helps explain the implementation or anticipates imminent
use.

```rust
fn releasedir(
    &self,
    _: &Request,
    inode: INodeNo,
    directory_handle: FileHandle,
    _: OpenFlags,
    reply: ReplyEmpty,
) {
    // ...
}
```

## Treat rustfmt as the layout authority

Run rustfmt and accept its stable output. Do not maintain hand formatting that
rustfmt immediately rewrites. Use names, types, and smaller expressions to
improve clarity when formatting alone cannot communicate the structure.

**kubefs status: applied.** Source layout follows `cargo fmt`; short function
signatures remain on one line when rustfmt chooses that representation.

## Keep the successful path flat

Use early returns, `?`, and `let ... else` when they make the main
operation read from top to bottom. Keep a `match` when its alternatives
represent meaningfully different outcomes; do not remove branching merely to
reduce line count.

**kubefs status: applied.** Private fallible operations return
`Result<_, FsError>` and use `?`. Pattern requirements such as read-only
open mode use `let ... else`.

```rust
let OpenAccMode::O_RDONLY = flags.acc_mode() else {
    return Err(FsError::WriteAccessRequested);
};
```

The model-to-file-type conversion remains an explicit `match` because the
variants represent distinct filesystem behavior.

## Separate fallible work from protocol responses

Boundary methods should translate inputs, delegate fallible work to ordinary
operations, and translate one final result into the protocol response. Helpers
that perform domain work should return values and errors rather than own a
one-shot response object.

**kubefs status: applied.** FUSE callbacks delegate to helpers such as
`lookup_attr`, `directory_entries`, `open_file_handle`, and
`release_file_handle`. Each callback then sends exactly one success or error
reply.

```rust
match self.release_file_handle(inode, file_handle) {
    Ok(()) => reply.ok(),
    Err(err) => reply.error(err.errno()),
}
```

Kubernetes access is behind the async `ClusterReader` boundary, while the
synchronous-to-async bridge remains in the adapter.

## Use ownership to communicate lifetime

Borrow when an operation only observes a value, consume when it may retain the
value, and share ownership explicitly when data must survive concurrent
removal from its container. Avoid cloning solely to satisfy an unnecessarily
broad interface.

**kubefs status: applied.** Registry lookup borrows a `Node`, while
`get_or_create_inode` consumes one because the registry may store it. Open
snapshots are returned as `Arc<OpenFile>`, allowing a read already in
progress to remain valid if `release` concurrently removes the table entry.

```rust
let file = self
    .open_files
    .get(file_handle)?
    .ok_or(FsError::FileHandleNotFound)?;
```

## Make errors semantic and standard

Error variants should describe meaningful failure categories rather than use
generic buckets such as `InvalidArgument(String)`. Error types should
implement `std::error::Error`, retain useful source information, and be
translated at the boundary where the required external error vocabulary is
known.

**kubefs status: applied.** `ClusterError`, registry errors, table errors,
and `FsError` derive `thiserror::Error`.
`ClusterError::CannotHaveChildren` and
`ExpectedNamespacedResource` describe distinct failures.
`FsError::errno` owns filesystem errno mapping.

```rust
impl From<ClusterError> for FsError {
    fn from(error: ClusterError) -> Self {
        match error {
            ClusterError::NotFound => FsError::NotFound,
            ClusterError::CannotHaveChildren => FsError::NotDirectory,
            other => FsError::Cluster(other),
        }
    }
}
```

## Check numeric representation boundaries

Use checked arithmetic and fallible conversions when values cross integer
representations or participate in externally visible offsets, identifiers, or
sizes. Give conversion failures a meaning appropriate to the operation rather
than applying a broad conversion from every integer error.

**kubefs status: applied.** Inode and handle allocation use `checked_add`,
directory cookies use fallible conversion plus checked addition, and object
length conversion maps explicitly to `FileSizeOverflow`.

```rust
let next_offset = index
    .checked_add(FIRST_CHILD_COOKIE)
    .ok_or(FsError::DirectoryCookieOverflow)?;
```

## Make concurrency and lock failure policy explicit

Keep critical sections small, do not return references tied to a released
lock, and decide deliberately whether poisoned state is recoverable. Shared
state errors should be represented rather than silently assumed impossible.

**kubefs status: applied.** Registry and handle-table mutex poisoning becomes
a typed error. Tables clone an `Arc` while holding the lock and release the
lock before callers access snapshot data.

```rust
let state = self.state.lock()?;
Ok(state.entries.get(&handle).cloned())
```

## Document every unsafe proof

Keep unsafe blocks as small as practical and precede them with a `// SAFETY:`
comment explaining why the operation's preconditions hold. The comment should
state the proof, not merely repeat the operation.

**kubefs status: applied.** Process credential calls are isolated and explain
why their foreign-function boundary is sound. Mount ownership is captured once
during `KubeFs` construction instead of invoking the unsafe boundary for every
attribute request.

```rust
// SAFETY: getuid and getgid take no arguments and have no caller-side
// memory-safety preconditions.
let (owner_uid, owner_gid) = unsafe { (libc::getuid(), libc::getgid()) };
```

For an `unsafe fn`, use a `# Safety` documentation section to state the
obligations imposed on its callers.

## Use structured, contextual diagnostics

Diagnostics should use structured fields with stable names. Keep the message
short and constant; put values such as the operation, typed identity, source
error, and outcome in fields rather than interpolating them into prose. This
keeps diagnostics useful to both people and machines and allows the output
format to change without changing instrumentation.

Choose levels according to the meaning and urgency of an event:

| Level | General meaning |
| --- | --- |
| `error` | An internal invariant was violated or state may be corrupted and requires investigation. |
| `warn` | An operation failed because of a recoverable external or operational problem. |
| `info` | A low-volume application lifecycle event useful during normal operation. |
| `debug` | An expected negative outcome or state transition useful while diagnosing behaviour. |
| `trace` | High-volume, request-by-request detail that is normally disabled. |

Log an error once, at the layer that handles its outcome and has enough
context to describe it. Lower layers should normally return typed errors
instead of logging and returning the same failure. Logging must not change
control flow, error mapping, externally visible behaviour, or cleanup.

Use formatting deliberately: `%value` records `Display`, while `?value`
records `Debug`. Record identifiers and request metadata, but do not record
credentials, authorization headers, configuration secrets, or potentially
sensitive payload contents. Successful high-frequency operations should not
produce `info` events. Keep each field's representation stable across events;
convert typed numeric identifiers to integers for native recording rather than
alternating between numeric and debug-formatted values.

Do not derive `Debug` automatically for a type that owns sensitive or
potentially large payload data. If diagnostics later require such a type to be
printable, provide a deliberate implementation that reports safe metadata
such as identity and length without including the payload.

Libraries should emit diagnostics but should not install a global subscriber.
The executable owns subscriber selection, filtering, formatting, and output.
Prefer fallible initialization when a subscriber may already be installed or
configuration may be invalid.

**kubefs status: applied.** The executable installs a fallible, environment-
filtered subscriber. FUSE callbacks record invocation, success, and classified
failure events, while the cluster boundary records requests. Helpers,
registries, handle tables, and cluster readers return typed errors instead of
logging the same failure again. `OpenFile` deliberately does not derive
`Debug`, because it owns serialized Kubernetes object data.

Use these fields consistently where they are available:

- `operation`
- `inode` or `parent_inode`
- `file_handle` or `directory_handle`
- `name`
- `offset` and `size`
- `errno`
- `error`

For current filesystem errors, use the following default classification:

- `debug`: invalid names, missing nodes, non-directory traversal, attempts to
  open a directory as a file, and write access rejected by the read-only
  filesystem.
- `warn`: Kubernetes API or transport failures and unknown file or directory
  handles.
- `error`: registry or handle-table failures, representational overflow,
  missing registered parents, and handle/inode mismatches.

An absent child is ordinary filesystem probing and must not be a warning. For
example, an editor looking for `.git` or `.gitignore` should result in an
`ENOENT` reply and, at most, a `debug` event.

```rust
tracing::warn!(
    operation = "fuse.read",
    inode = u64::from(inode),
    file_handle = u64::from(file_handle),
    ?errno,
    error = %error,
    "FUSE request failed",
);
```

Mounting, receiving a shutdown signal, and completing unmount are suitable
`info` events. Callback invocations and successful reads or directory pages
belong at `trace`; successful handle allocation and release may be `debug`.
Kubernetes object data and authentication material must never be logged.

## Keep application composition explicit and fallible

Construct long-lived dependencies in one visible place, make configuration an
input, and propagate setup failures. Avoid global mutable initialization and
production `unwrap()` when setup can return an ordinary error.

**kubefs status: applied.** The executable parses a required mount point,
constructs a locally owned fallible Tokio runtime, installs tracing, reports
lifecycle events, and maps the application result to `ExitCode`. Future
filesystem policy flags should be parsed here and converted into internal
configuration types rather than coupling filesystem code to Clap.

```rust
#[derive(clap::Parser)]
struct Args {
    mount_point: std::path::PathBuf,
}
```

## Keep dependencies and tooling intentional

Add a dependency together with the code that needs it, select only required
features, and periodically remove unused crates and features. Formatting and
static analysis should be reproducible project-wide.

**kubefs status: partially applied; TODO.** Direct dependencies and their
features have been reviewed: unused `futures`, schema generation, experimental
fuser support, and excess Tokio features were removed. The current code passes
`cargo fmt --check` and strict Clippy. Add these checks to CI after the active
filesystem-semantics milestone.

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
```
