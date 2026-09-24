# Filesystem Semantics

`kubefs` presents a changing Kubernetes API through a read-only filesystem.
Kubernetes objects do not naturally have all the semantics of disk-backed
files, so this document defines the behaviour that users can rely on.

## Object consistency modes

The planned CLI exposes an object consistency policy:

```text
--object-consistency=stable-open
--object-consistency=live
```

`stable-open` is the default. The setting affects regular files representing
Kubernetes objects. Directory handles use stable directory snapshots in both
modes.

### Stable-open

Resolving an object captures its UID, `resourceVersion`, and serialized
contents from one Kubernetes API response. That immutable revision receives an
inode. Every open of that inode observes the same bytes and attributes, even if
the Kubernetes object is updated or deleted afterward.

A later path lookup may observe a different `resourceVersion` and therefore
resolve the path to a new inode. Existing handles continue referring to the
old revision inode. This models a Kubernetes update as atomic replacement
rather than in-place mutation.

The current implementation still captures bytes independently for each open
handle and can therefore associate different snapshots with the same inode.
It does not satisfy the final stable-open semantics yet.

### Live

All handles for an object refer to shared, inode-scoped contents. An update to
the same Kubernetes object replaces those contents, and an already open
handle may observe the new version on a later read. This resembles an
out-of-band modification of an existing disk inode.

Each individual read callback must hold one immutable content value for the
duration of the callback. Separate reads may observe different Kubernetes
versions, so a file read in several chunks is not guaranteed to be a coherent
serialization if the object changes concurrently. This is an intentional
property of live mode.

Live contents should be shared by inode rather than fetched and stored
independently for every open handle. A cached value should record the
Kubernetes `resourceVersion` that produced it, and its refresh policy must be
explicit.

In live mode, `getattr` reports attributes for the current inode contents. It
does not report a handle-specific historical size.

## Object identity and replacement

Kubernetes UID distinguishes an object across deletion and recreation, while
`resourceVersion` distinguishes observations of that object:

- In stable-open mode, the same UID with a new `resourceVersion` becomes a new
  immutable revision inode.
- In a future live mode, the same UID with a new `resourceVersion` updates the
  shared contents of the existing inode.
- A different UID appearing at the same API path is a new object in either
  mode.
- A missing UID means the object has been deleted from the directory.

Path identity and revision identity must remain separate: the API path is used
to resolve the current object, while UID plus `resourceVersion` identifies the
immutable snapshot stored for an inode. Resource versions are opaque strings;
the filesystem compares them only for equality and does not interpret their
ordering.

The inode registry does not yet implement revision identity. Until it does,
the filesystem cannot provide exact replacement semantics.

## Snapshot storage and resource cost

The planned stable-open implementation supports:

```text
--snapshot-store=memory
--snapshot-store=disk
```

Memory is the default so Kubernetes object data is not written to local disk
without explicit consent. The selected store changes resource usage, not
externally visible file contents or inode semantics.

The memory store retains immutable serialized object revisions in process
memory. Recursive traversal or metadata inspection can resolve many objects,
potentially approaching the total serialized size of every object visible to
the Kubernetes identity.

The disk store retains uncompressed immutable snapshots and serves reads from
those files. It reduces Rust heap use but consumes local disk space and I/O;
the operating system may also cache file pages in memory. Because snapshots
can contain secrets, the cache directory and files require private
permissions, an explicit location and capacity policy, and cleanup after
normal shutdown or crashes.

Compression is intentionally out of scope. It complicates positional reads
and would require whole-object decompression or an indexed format.

An inode snapshot remains required while the kernel holds lookup references,
an open file handle refers to it, or an open directory snapshot pins it. These
references can legitimately persist indefinitely, so neither storage mode can
promise bounded use without configured limits. Unreferenced revisions may be
evicted; referenced revisions must not be silently discarded merely because a
time limit expired.

The initial cleanup implementation should reclaim registry records and
snapshot storage without reusing numeric inode values. If inode numbers are
later reused, each reuse must receive a new FUSE generation.

## Deletion and open handles

Path resolution after deletion should return `ENOENT`. As in a conventional
Unix filesystem, deletion should not invalidate a handle that was already
open:

- In stable-open mode, the handle continues reading its captured snapshot.
- In live mode, the inode retains its last known contents until its final open
  handle is released.

If another object is created at the same path with a different UID, new
lookups and opens should resolve to a new inode while handles for the deleted
object continue to refer to the old inode.

## Directory consistency

`opendir` captures an ordered directory snapshot. `readdir` uses that
snapshot and stable continuation cookies until `releasedir`, regardless of
the selected object consistency mode. Changes in the cluster become visible
through a newly opened directory handle.

This prevents additions or removals between paginated `readdir` calls from
causing skipped, duplicated, or invalid entries.

## Read-only behaviour

The mount is read-only. Operations that attempt to mutate filesystem state
should fail with `EROFS`. Read-only open, lookup, attribute, directory, and
synchronization operations should follow conventional filesystem behaviour
where the Kubernetes projection permits it.

Permission checking, attribute caching, object refresh intervals, inode
invalidation, and the exact treatment of transient Kubernetes failures remain
separate policies and must be documented before their implementations are
considered complete.
