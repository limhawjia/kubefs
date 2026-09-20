# kubefs TODO

This is an experimental read-only FUSE view of a Kubernetes API. Correct FUSE
behaviour, bounded resource use, and clear consistency semantics come before
new features.

## Current layout policy

For a namespaced resource, its directory shows only namespaces that currently
contain at least one object visible to the caller. The `namespaces` resource is
the separate view for enumerating all namespaces. This is an intentional
projection, not a completeness guarantee for each resource directory.

## Current milestone order

Focus the next work on these milestones, in order:

1. [x] Add stable per-open directory snapshots with `opendir`, `readdir`, and
   `releasedir`.
2. [x] Replace ad-hoc or silent callback diagnostics with structured logging.
3. [x] Replace the hard-coded application setup with an explicit CLI and locally
   owned fallible Tokio runtime.
4. [ ] Iron out filesystem semantics, including attributes, errno choices,
   read-only mutation behaviour, and externally visible consistency rules.

The active filesystem-semantics milestone is divided into these steps:

1. [x] Record the selected object consistency models and directory, update,
   replacement, and deletion semantics in
   [`FILESYSTEM_SEMANTICS.md`](FILESYSTEM_SEMANTICS.md).
2. [ ] Add a typed filesystem configuration and expose
   `--object-consistency=stable-open|live`, defaulting to `stable-open`.
   Thread the setting into `KubeFs` without changing behaviour first.
3. [ ] Make the existing per-open snapshot implementation explicitly satisfy
   the documented stable-open mode, including handle-aware attributes.
4. [ ] Add inode-scoped object contents and an explicit refresh policy for
   live mode. Each read callback must use one immutable content value, while
   separate reads may observe different Kubernetes versions.
5. [ ] Use Kubernetes UID as object/inode identity so an update retains its
   inode while deletion followed by recreation creates a new inode. Preserve
   deleted inode contents until the last open handle is released.
6. [ ] Finish conventional read-only filesystem behaviour: attribute block
   accounting, time policy, explicit `EROFS` mutation callbacks, sync/access
   behaviour, and documented errno choices.

All other incomplete items below are intentionally deferred until these four
milestones are complete and their design trade-offs can be reconsidered.

## Immediate review follow-ups

- [x] Separate the displayed core-group name (`"core"`) from its Kubernetes
  API-group value (`""`) when constructing a dynamic `ApiResource`.
- [x] Keep identity ordering consistent with equality: `ApiResource` ordering
  must include every field used by its `Eq` implementation, including kind and
  scope.
- [x] Keep `ApiGroup` ordering consistent with equality when a discovered
  group is named `"core"`; it must distinguish that named group from the
  virtual core group after comparing their displayed names.
- [x] Make `ClusterReader::child` resolve an object without first listing all
  of its siblings; enumeration belongs only in `children`.
- [x] Expose only top-level Kubernetes resources that support both `list` and
  `get`, applying the same discovery predicate to enumeration and direct
  lookup so create-only APIs do not become unusable directories.
- [x] Sort the namespace names produced by the sparse namespaced-resource
  projection after de-duplication.
- [x] Preserve lexicographic ordering of visible API-group directory names:
  derived enum ordering currently puts `Core` before every named group.

## 1. Refactor the domain model

- [x] Create `model.rs` and move the filesystem identity types there.
- [x] Model the core Kubernetes API group as a variant (`ApiGroup::Core`), not
  the display string `"core"`.
- [x] Keep Kubernetes identities as UTF-8 `String`s. Convert to `OsStr`/
  `OsString` only at the FUSE boundary; remove impossible non-UTF-8 error paths
  from the domain model.
- [x] Make resource scope (`Cluster` or `Namespaced`) part of the discovered
  `ApiResource`, rather than rediscovering it later.
- [x] Give `NamespaceId` and object identity their own named structs when they
  have behaviour or are reused; keep `Node` as the enum representing a virtual
  filesystem location.
- [x] Keep fields private where constructors or discovery must uphold an
  invariant.

## 2. Separate cluster access from FUSE

- [x] Create a `cluster` module with a narrow async `ClusterReader` trait and
  `ClusterError` type.
- [x] Move Kubernetes API discovery, dynamic-object, HTTP request, and
  Kubernetes-error conversion code into `cluster::kube`. Application
  composition may still accept a `kube::Client`.
- [x] Have `ClusterReader` return domain `Node`s and object bytes, never FUSE
  replies, inodes, or raw Kubernetes discovery values.
- [x] Include distinct `children(parent)` and `child(parent, name)` operations:
  enumeration may list a directory, while lookup must not list every object to
  find one name.
- [x] Keep FUSE errno mapping in the FUSE module, where filesystem semantics
  belong.
- [x] Ensure the FUSE module has no `kube::` imports and the cluster module has
  no `fuser::` imports.

The new `KubeFs` is wired into the executable and depends on `ClusterReader`
and domain `Node`s. The old `fs.rs` and `registry.rs` files are legacy code and
are no longer compiled.

## 3. Reshape the FUSE adapter

- [x] Keep `KubeFs` as a thin adapter: resolve an inode, invoke
  `ClusterReader`, build attributes, map errors to errno, and send one reply.
- [x] Move the inode registry into a focused `fuse::inode` module. Make its
  public API consistently use `INodeNo`, preserve root inode `1`, and handle
  allocation exhaustion explicitly.
- [x] Add an `OpenFileTable` keyed by FUSE file handle; this owns a stable
  content snapshot for each open object file.
- [x] Keep the synchronous-to-async bridge (`Handle::block_on`) in the FUSE
  adapter only. Cluster methods themselves must remain `async fn`s.
- [x] Remove duplicated node traversal between `lookup` and `readdir` by
  sharing domain-level child resolution, not reply objects.

## 4. Cleanup checkpoint

- [x] Make `OpenFile` fields private and expose narrow immutable accessors.
- [x] Have `release` verify that the removed snapshot belongs to the callback
  inode before replying successfully.
- [x] Extract the repeated `Node`-to-`FileType` mapping used by `readdir` and
  `getattr` into one FUSE-local helper.
- [x] Centralize `ClusterError`-to-errno mapping so callbacks handle
  authorization and unexpected failures consistently and error payloads are
  not discarded.

## 5. Add lifetime and cache policies

- [ ] Add `forget`-based inode reference tracking and eviction. The registry
  must not retain every object ever seen by a long-running mount.
- [ ] Stop downloading full object YAML from `getattr`. `stat` must not make a
  full-content request.
- [x] Fetch object YAML once at `open`, retain it in the open-file table, and
  serve all reads for that handle from the snapshot. Define its `release`
  behaviour.
- [x] Add `opendir`/`releasedir` and a per-open directory snapshot so
  `readdir` cookies remain meaningful if the Kubernetes listing changes
  between paginated calls.
- [ ] Add a discovery cache for groups, versions, resources, namespaces, and
  object names with an explicit refresh policy.
- [ ] Define whether `getattr` revalidates registry-backed nodes against the
  cluster. Object attributes currently check existence incidentally while
  fetching content, but directory-like nodes are trusted once registered;
  decide on cache-backed revalidation and whether disappearance returns
  `ENOENT` or `ESTALE`.
- [ ] Implement the external-change semantics documented in
  [`FILESYSTEM_SEMANTICS.md`](FILESYSTEM_SEMANTICS.md). Object read modes and
  deletion behaviour are defined; refresh TTLs, stale directory entries,
  permissions, and transient failure policy still need concrete decisions.

## 6. Complete filesystem correctness

- [x] Make `read` safe for every offset and size. Return an empty response at
  EOF; never slice with an offset beyond the fetched data or allow integer
  conversion/addition to overflow.
- [x] Emit `.` and `..` from `readdir` and use valid continuation cookies.
- [x] Give regular files `nlink = 1` and directories `nlink = 2`.
- [ ] Calculate attribute `blocks` from content size and document the
  time-field policy.
- [ ] Exclude `metadata.managedFields` from object YAML by default and expose
  an `--include-managed-fields` CLI flag to retain it when requested. Thread
  this choice through typed filesystem configuration; attribute sizes and file
  snapshots must use the same selected representation returned by `read`.
- [x] Remove production-path `unwrap()` calls. Propagate or explicitly map
  failures from runtime setup, mounting, request construction, header parsing,
  and Kubernetes metadata.
- [ ] Translate Kubernetes errors to useful errno values: deleted/missing
  resources to `ENOENT`, authorization failures to `EACCES`/`EPERM`, and
  unexpected service failures to `EIO`.
- [x] Reject writable `open` requests explicitly rather than relying solely on
  the read-only mount option.
- [ ] Add explicit read-only handling for mutation callbacks.

## 7. Deferred tests and polish

- [ ] Unit-test registry allocation, reverse lookup, eviction, and root inode
  invariants.
- [ ] Unit-test `read` offsets: zero, middle, exact EOF, past EOF, and large
  offsets/sizes.
- [ ] Unit-test directory cookie pagination, including `.` and `..`.
- [ ] Unit-test Kubernetes-error-to-errno conversion.
- [ ] Add fake-cluster integration tests covering lookup, readdir, getattr,
  and read without requiring a live Kubernetes cluster.
- [ ] Add CI for `cargo fmt --check`, `cargo clippy --all-targets
  --all-features -- -D warnings`, and `cargo test --all-features`.
- [x] Use structured logging (`tracing`) with operation, inode/path, and error
  context instead of `println!`/`eprintln!`.
- [x] Remove unused dependencies and dependency features from the compiled
  implementation.
- [ ] Remove the uncompiled legacy `fs.rs` and `registry.rs` after confirming
  the replacement path is feature-complete; their unused `RegistryError` is
  removed with them.
- [x] Replace the hard-coded mount path with a required CLI argument and use a
  local fallible Tokio runtime instead of the global runtime `unwrap()`.
- [x] Add a README describing the virtual filesystem layout, current
  behaviour, limitations, and development commands.
- [ ] Add package metadata such as `rust-version`, a licence, and repository
  URL.

## Fun extensions

- [ ] Explore an ownership-oriented view that models Kubernetes parent-child
  relationships from `metadata.ownerReferences` instead of exposing only the
  flat API group/version/resource hierarchy. Define how the filesystem handles
  multiple owners, missing or inaccessible owners, namespace and scope
  boundaries, ownership cycles, and objects without an owner before choosing
  paths or inode identities.
