# kubefs

`kubefs` is an experimental read-only FUSE view of a Kubernetes cluster API.
It projects API discovery, namespaces, and objects into a filesystem so that
cluster objects can be browsed and read as YAML files.

This is a learning project focused on Rust, FUSE semantics, Kubernetes API
design, and maintainable boundaries. It is not ready for production use.

## Filesystem layout

The root contains the virtual `core` API group and the named API groups visible
to the current Kubernetes identity. Beneath each group, the hierarchy follows
Kubernetes discovery:

```text
/
└── <api-group>/
    └── <api-version>/
        └── <resource>/
            ├── <cluster-scoped-object>
            └── <namespace>/
                └── <namespaced-object>
```

Object files contain the YAML representation returned by the Kubernetes API.
For a namespaced resource, only namespaces containing at least one visible
object are shown. The Kubernetes `namespaces` resource remains the independent
view of every namespace visible to the caller.

## Current behaviour

- The mount and projected permissions are read-only.
- Each open object handle reads from the object snapshot captured by `open`.
- Each open directory handle uses a stable, ordered entry snapshot until
  `releasedir`.
- Kubernetes access uses the credentials and context selected by `kube`'s
  standard client configuration.
- FUSE callbacks and application lifecycle events use structured `tracing`
  diagnostics.

The current object snapshot implementation provides stable reads but does not
yet provide complete conventional inode replacement semantics. The planned
`stable-open` and `live` policies, including update and deletion behaviour, are
defined in [Filesystem semantics](docs/FILESYSTEM_SEMANTICS.md). The CLI flag
for selecting between those policies has not been implemented yet.

## Snapshot storage and cost

> [!WARNING]
> The planned stable-open model gives each observed Kubernetes object revision
> an immutable inode snapshot. Recursive tools such as `rg`, `find`, or an
> editor indexer can resolve a large part of the cluster and may cause many
> object bodies to be retained. In memory mode, usage can approach the total
> serialized size of all objects visible to the Kubernetes identity.

The planned CLI will allow the snapshot backend to be selected explicitly:

```text
--snapshot-store=memory
--snapshot-store=disk
```

Memory will remain the default so cluster data is not written to disk without
the user's consent. Disk mode will trade heap use for local disk capacity and
I/O. It will store uncompressed snapshots so FUSE reads can efficiently access
arbitrary offsets.

> [!CAUTION]
> Disk snapshots may contain Secrets and other sensitive Kubernetes objects.
> The disk store must use a private cache directory and files, enforce a
> capacity limit, and define cleanup for both normal shutdown and stale data
> left by crashes. The option is documented here as planned behavior and is
> not implemented yet.

Open file handles, kernel lookup references, and open directory snapshots can
legitimately retain an object revision indefinitely. Resource limits may
reject new snapshots when all retained revisions are still referenced; they
must not silently discard data required by existing handles.

## Requirements

- Rust with support for edition 2024
- A FUSE-capable system with the required userspace mount tooling
- Access to a Kubernetes cluster through the usual kubeconfig or in-cluster
  configuration
- Kubernetes permissions for the discovery, metadata, and object reads to be
  exposed

The mounted view can expose sensitive cluster data. Its contents are limited
by the Kubernetes identity used to create the client, so that identity should
have only the permissions intended for filesystem users.

## Run

Create an empty mount point and pass it as the required positional argument:

```bash
mkdir -p /tmp/kubefs-mount
cargo run -- /tmp/kubefs-mount
```

Press Ctrl-C to request unmount and shutdown.

Logging is controlled through `RUST_LOG`. For callback-level diagnostics:

```bash
RUST_LOG=kubefs=trace cargo run -- /tmp/kubefs-mount
```

## Development checks

```bash
cargo fmt --check
cargo check
cargo clippy --all-targets --all-features -- -D warnings
```

Unit and integration tests are currently deferred for this learning project.

## Project documentation

- [Implementation roadmap](docs/TODO.md)
- [Code style guide](docs/STYLE_GUIDE.md)
- [Filesystem semantics](docs/FILESYSTEM_SEMANTICS.md)

The source is organized around a domain model, Kubernetes cluster access, the
FUSE adapter, inode state, and open-handle state. These boundaries are
described in the style guide and tracked in the roadmap.
