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

Opening an object captures its serialized contents. Every read through that
file handle observes the same bytes, even if the Kubernetes object is updated
or deleted afterward. A later open may capture a newer object version.

This mode provides a coherent byte stream across multiple reads. It resembles
opening an old inode before a pathname is atomically replaced, although the
current implementation does not yet assign a distinct inode to each object
version. Consequently, two handles for the same reported inode may contain
different snapshots.

When a file handle is supplied to `getattr`, attributes that depend on file
contents, especially `size`, should describe that handle's snapshot. A
path-based lookup or `getattr` without a handle should describe the currently
resolved Kubernetes object.

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

Kubernetes object UID is the closest analogue to filesystem inode identity:

- The same UID with a new `resourceVersion` is an update to the same object.
- A different UID appearing at the same API path is a replacement object.
- A missing UID means the object has been deleted from the directory.

The inode registry does not yet implement UID-based identity. Until it does,
the filesystem cannot perfectly distinguish an update from deletion followed
by recreation at the same path. UID-aware inode identity is therefore a
prerequisite for exact replacement semantics, not a guarantee of the current
implementation.

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
