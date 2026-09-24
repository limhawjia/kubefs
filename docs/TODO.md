# kubefs TODO

This roadmap contains remaining work only. Completed architecture and behaviour
are described in the [README](../README.md),
[style guide](STYLE_GUIDE.md), and
[filesystem semantics](FILESYSTEM_SEMANTICS.md), with implementation history
retained by Git.

Correct FUSE behaviour, bounded resource use, and explicit consistency
semantics take priority over new views and features.

## Current milestone: stable-open revision semantics

Implement true replacement semantics before adding the optional live-update
model:

1. [ ] Keep Kubernetes path identity separate from an `ObjectRevision`
   containing the object's UID and opaque `resourceVersion`. A new revision of
   the same object must receive a different inode in stable-open mode.
2. [ ] Resolve an object's identity and serialized bytes from the same
   Kubernetes API response. Directory enumeration may remain metadata-only,
   but object lookup must produce an immutable revision snapshot rather than
   relying on a later historical GET.
3. [ ] Replace the inode `BiMap` with records that track kernel lookup counts,
   open-handle pins, and directory-snapshot pins. Implement `forget` and evict
   a revision only after every reference class reaches zero.
4. [ ] Introduce a snapshot-store boundary with memory and uncompressed disk
   implementations. Expose `--snapshot-store=memory|disk`, defaulting to
   `memory` so Kubernetes data is not written locally without explicit
   consent. Give both stores configurable capacity limits; allow a private
   disk-cache location to be selected.
5. [ ] Make every open handle share the immutable snapshot owned by its
   revision inode. Attributes and reads for that inode must use the same stored
   representation without another Kubernetes request; `getattr` must no
   longer download full object YAML.
6. [ ] Clean up the disk snapshot store after orderly shutdown. Remove only
   the cache instance owned by the current mount, never a user-supplied parent
   directory; define how a later startup identifies and handles stale instance
   directories left by crashes.
7. [ ] Initially evict registry records and snapshot storage without reusing
   numeric inode values. After eviction is correct, add an inode free pool and
   assign a new FUSE generation whenever a reclaimed number is reused.

## Filesystem correctness and cache policy

- [ ] Calculate attribute `blocks` from content size and document the
  timestamp policy.
- [ ] Exclude `metadata.managedFields` from object YAML by default and expose
  an `--include-managed-fields` CLI flag to retain it. Thread this choice
  through typed configuration; attributes, stored snapshots, and reads must
  use the same selected representation.
- [ ] Translate Kubernetes errors to useful errno values: missing resources to
  `ENOENT`, authorization failures to `EACCES` or `EPERM`, capacity exhaustion
  to a documented resource error, and unexpected service failures to `EIO`.
- [ ] Add explicit `EROFS` handling for mutation callbacks. Define conventional
  read-only behaviour for access and synchronization callbacks.
- [ ] Add a discovery cache for groups, versions, resources, namespaces, and
  object names with an explicit refresh policy.
- [ ] Define whether `getattr` revalidates directory-like registry nodes
  against the cluster. Decide how TTL expiry, stale entries, deletion,
  transient Kubernetes failures, and `ENOENT` versus `ESTALE` interact.
- [ ] Add inode-scoped mutable contents and an explicit refresh policy for a
  later live mode. Expose `--object-consistency=stable-open|live` only after
  both choices have real implementations; default to `stable-open`.

## Deferred maintenance and polish

- [ ] Unit-test registry allocation, reverse lookup, reference accounting,
  eviction, generation reuse, and root inode invariants.
- [ ] Unit-test read offsets at zero, the middle, exact EOF, past EOF, and
  representation limits.
- [ ] Unit-test directory-cookie pagination, including `.` and `..`.
- [ ] Unit-test Kubernetes-error-to-errno conversion.
- [ ] Add fake-cluster integration tests covering lookup, readdir, getattr,
  open, read, release, forget, and revision replacement without a live
  Kubernetes cluster.
- [ ] Add CI for `cargo fmt --check`, `cargo clippy --all-targets
  --all-features -- -D warnings`, and `cargo test --all-features`.
- [ ] Remove the uncompiled legacy `fs.rs` and `registry.rs` after confirming
  the replacement path is feature-complete; their unused `RegistryError` is
  removed with them.
- [ ] Add package metadata such as `rust-version`, a licence, and repository
  URL.

## Fun extensions

- [ ] Explore an ownership-oriented view that models Kubernetes parent-child
  relationships from `metadata.ownerReferences` instead of exposing only the
  flat API group/version/resource hierarchy. Define how the filesystem handles
  multiple owners, missing or inaccessible owners, namespace and scope
  boundaries, ownership cycles, and objects without an owner before choosing
  paths or inode identities.
