use std::{
    ffi::{OsStr, OsString},
    sync::Arc,
    time::{Duration, SystemTime},
};

use fuser::{
    Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, Generation, INodeNo, LockOwner,
    OpenAccMode, OpenFlags, ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty, ReplyEntry,
    ReplyOpen, Request,
};
use thiserror::Error;
use tokio::runtime::Handle;

use super::{
    directorytable::{DirectoryEntry, OpenDirectory, OpenDirectoryTable, OpenDirectoryTableError},
    filetable::{OpenFile, OpenFileTable, OpenFileTableError},
    inode::{InodeRegistry, InodeRegistryError},
};
use crate::{
    cluster::{ClusterError, ClusterReader},
    model::Node,
};

const CACHE_TTL: Duration = Duration::from_secs(1);
const INODE_GENERATION: Generation = Generation(1);
const BLOCK_SIZE: u32 = 512;

const DOT_COOKIE: u64 = 1;
const DOT_DOT_COOKIE: u64 = 2;
const FIRST_CHILD_COOKIE: u64 = 3;

#[derive(Clone, Copy)]
enum DiagnosticLevel {
    Debug,
    Warn,
    Error,
}

#[derive(Debug, Error)]
enum FsError {
    #[error("name is not valid UTF-8")]
    InvalidName,

    #[error("node is not a directory")]
    NotDirectory,

    #[error("node was not found")]
    NotFound,

    #[error("cluster operation failed: {0}")]
    Cluster(ClusterError),

    #[error("inode registry operation failed: {0}")]
    Registry(InodeRegistryError),

    #[error("file size cannot be represented")]
    FileSizeOverflow,

    #[error("parent node is not registered")]
    ParentNotRegistered,

    #[error("directory cookie cannot be represented")]
    DirectoryCookieOverflow,

    #[error("write access was requested on a read-only filesystem")]
    WriteAccessRequested,

    #[error("node is a directory")]
    IsDirectory,

    #[error("open-file table operation failed: {0}")]
    OpenFiles(OpenFileTableError),

    #[error("file handle not found")]
    FileHandleNotFound,

    #[error("file handle belongs to inode {stored_inode}, not requested inode {requested_inode}")]
    FileHandleInodeMismatch {
        requested_inode: INodeNo,
        stored_inode: INodeNo,
    },

    #[error("open-directory table operation failed: {0}")]
    OpenDirectories(OpenDirectoryTableError),

    #[error("directory handle not found")]
    DirectoryHandleNotFound,

    #[error(
        "directory handle belongs to inode {stored_inode}, not requested inode {requested_inode}"
    )]
    DirectoryHandleInodeMismatch {
        requested_inode: INodeNo,
        stored_inode: INodeNo,
    },
}

impl FsError {
    fn errno(&self) -> Errno {
        match self {
            FsError::InvalidName => Errno::EINVAL,
            FsError::NotDirectory => Errno::ENOTDIR,
            FsError::NotFound => Errno::ENOENT,
            FsError::Cluster(_) | FsError::Registry(_) => Errno::EIO,
            FsError::FileSizeOverflow => Errno::EOVERFLOW,
            FsError::ParentNotRegistered => Errno::EIO,
            FsError::DirectoryCookieOverflow => Errno::EOVERFLOW,
            FsError::WriteAccessRequested => Errno::EROFS,
            FsError::IsDirectory => Errno::EISDIR,
            FsError::OpenFiles(_) => Errno::EIO,
            FsError::FileHandleNotFound => Errno::EBADF,
            FsError::FileHandleInodeMismatch { .. } => Errno::EIO,
            FsError::OpenDirectories(_) => Errno::EIO,
            FsError::DirectoryHandleNotFound => Errno::EBADF,
            FsError::DirectoryHandleInodeMismatch { .. } => Errno::EIO,
        }
    }

    fn diagnostic_level(&self) -> DiagnosticLevel {
        match self {
            FsError::InvalidName
            | FsError::NotDirectory
            | FsError::NotFound
            | FsError::WriteAccessRequested
            | FsError::IsDirectory => DiagnosticLevel::Debug,

            FsError::Cluster(ClusterError::Api(_))
            | FsError::FileHandleNotFound
            | FsError::DirectoryHandleNotFound => DiagnosticLevel::Warn,

            FsError::Cluster(_)
            | FsError::Registry(_)
            | FsError::FileSizeOverflow
            | FsError::ParentNotRegistered
            | FsError::DirectoryCookieOverflow
            | FsError::OpenFiles(_)
            | FsError::FileHandleInodeMismatch { .. }
            | FsError::OpenDirectories(_)
            | FsError::DirectoryHandleInodeMismatch { .. } => DiagnosticLevel::Error,
        }
    }
}

impl From<InodeRegistryError> for FsError {
    fn from(err: InodeRegistryError) -> Self {
        FsError::Registry(err)
    }
}

impl From<ClusterError> for FsError {
    fn from(err: ClusterError) -> Self {
        match err {
            ClusterError::NotFound => FsError::NotFound,
            ClusterError::CannotHaveChildren => FsError::NotDirectory,
            other => FsError::Cluster(other),
        }
    }
}

impl From<OpenFileTableError> for FsError {
    fn from(err: OpenFileTableError) -> Self {
        FsError::OpenFiles(err)
    }
}

impl From<OpenDirectoryTableError> for FsError {
    fn from(err: OpenDirectoryTableError) -> Self {
        FsError::OpenDirectories(err)
    }
}

macro_rules! log_fs_error {
    ($error:expr, $($fields:tt)*) => {{
        let error = $error;
        match error.diagnostic_level() {
            DiagnosticLevel::Debug => tracing::debug!(
                $($fields)*
                error = %error,
                "FUSE request failed",
            ),
            DiagnosticLevel::Warn => tracing::warn!(
                $($fields)*
                error = %error,
                "FUSE request failed",
            ),
            DiagnosticLevel::Error => tracing::error!(
                $($fields)*
                error = %error,
                "FUSE request failed",
            ),
        }
    }};
}

pub struct KubeFs {
    reader: Box<dyn ClusterReader>,
    registry: InodeRegistry,
    open_files: OpenFileTable,
    open_directories: OpenDirectoryTable,
    runtime_handle: Handle,
    owner_uid: u32,
    owner_gid: u32,
    mount_time: SystemTime,
}

impl Filesystem for KubeFs {
    fn lookup(&self, _: &Request, parent_inode: INodeNo, name: &OsStr, reply: ReplyEntry) {
        tracing::trace!(
            operation = "fuse.lookup",
            parent_inode = u64::from(parent_inode),
            ?name,
            "FUSE callback invoked",
        );

        match self.lookup_attr(parent_inode, name) {
            Ok(attr) => {
                tracing::trace!(
                    operation = "fuse.lookup",
                    parent_inode = u64::from(parent_inode),
                    ?name,
                    inode = u64::from(attr.ino),
                    kind = ?attr.kind,
                    "FUSE callback succeeded",
                );

                reply.entry(&CACHE_TTL, &attr, INODE_GENERATION);
            }
            Err(err) => {
                let errno = err.errno();

                log_fs_error!(
                    &err,
                    operation = "fuse.lookup",
                    parent_inode = u64::from(parent_inode),
                    ?name,
                    ?errno,
                );

                reply.error(errno);
            }
        }
    }

    fn getattr(&self, _: &Request, inode: INodeNo, _: Option<FileHandle>, reply: ReplyAttr) {
        tracing::trace!(
            operation = "fuse.getattr",
            inode = u64::from(inode),
            "FUSE callback invoked",
        );

        match self.getattr_attr(inode) {
            Ok(attr) => {
                tracing::trace!(
                    operation = "fuse.getattr",
                    inode = u64::from(attr.ino),
                    kind = ?attr.kind,
                    size = attr.size,
                    "FUSE callback succeeded",
                );

                reply.attr(&CACHE_TTL, &attr);
            }
            Err(err) => {
                let errno = err.errno();
                log_fs_error!(
                    &err,
                    operation = "fuse.getattr",
                    inode = u64::from(inode),
                    ?errno,
                );

                reply.error(errno);
            }
        }
    }

    fn opendir(&self, _: &Request, inode: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        tracing::trace!(
            operation = "fuse.opendir",
            inode = u64::from(inode),
            ?flags,
            "FUSE callback invoked",
        );

        match self.open_directory_handle(inode, flags) {
            Ok(directory_handle) => {
                tracing::trace!(
                    operation = "fuse.opendir",
                    inode = u64::from(inode),
                    ?flags,
                    directory_handle = u64::from(directory_handle),
                    "FUSE callback succeeded",
                );

                reply.opened(directory_handle, FopenFlags::empty());
            }
            Err(err) => {
                let errno = err.errno();
                log_fs_error!(
                    &err,
                    operation = "fuse.opendir",
                    inode = u64::from(inode),
                    ?flags,
                    ?errno,
                );

                reply.error(errno);
            }
        }
    }

    fn readdir(
        &self,
        _: &Request,
        inode: INodeNo,
        directory_handle: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        tracing::trace!(
            operation = "fuse.readdir",
            inode = u64::from(inode),
            directory_handle = u64::from(directory_handle),
            offset,
            "FUSE callback invoked",
        );

        match self.directory_snapshot(inode, directory_handle) {
            Ok(directory) => {
                let entries = directory.entries();

                let mut buffer_full = false;
                let mut entries_added = 0usize;
                for entry in entries.iter().filter(|entry| entry.next_offset() > offset) {
                    let entry_inode = entry.inode();
                    let entry_offset = entry.next_offset();
                    let entry_kind = entry.kind();
                    let entry_name = entry.name();

                    if reply.add(entry_inode, entry_offset, entry_kind, entry_name) {
                        buffer_full = true;
                        break;
                    }

                    entries_added += 1;

                    tracing::trace!(
                        operation = "fuse.readdir.entry",
                        directory_handle = u64::from(directory_handle),
                        entry_inode = u64::from(entry_inode),
                        entry_offset,
                        entry_kind = ?entry_kind,
                        entry_name = ?entry_name,
                        "directory entry added to reply",
                    );
                }

                tracing::trace!(
                    operation = "fuse.readdir",
                    inode = u64::from(inode),
                    directory_handle = u64::from(directory_handle),
                    offset,
                    entries_added,
                    buffer_full,
                    "FUSE callback succeeded",
                );

                reply.ok();
            }
            Err(err) => {
                let errno = err.errno();

                log_fs_error!(
                    &err,
                    operation = "fuse.readdir",
                    inode = u64::from(inode),
                    directory_handle = u64::from(directory_handle),
                    offset,
                    ?errno,
                );

                reply.error(errno);
            }
        }
    }

    fn releasedir(
        &self,
        _: &Request,
        inode: INodeNo,
        directory_handle: FileHandle,
        _: OpenFlags,
        reply: ReplyEmpty,
    ) {
        tracing::trace!(
            operation = "fuse.releasedir",
            inode = u64::from(inode),
            directory_handle = u64::from(directory_handle),
            "FUSE callback invoked",
        );

        match self.release_directory_handle(inode, directory_handle) {
            Ok(()) => {
                tracing::trace!(
                    operation = "fuse.releasedir",
                    inode = u64::from(inode),
                    directory_handle = u64::from(directory_handle),
                    "FUSE callback succeeded",
                );

                reply.ok();
            }
            Err(err) => {
                let errno = err.errno();

                log_fs_error!(
                    &err,
                    operation = "fuse.releasedir",
                    inode = u64::from(inode),
                    directory_handle = u64::from(directory_handle),
                    ?errno,
                );

                reply.error(errno);
            }
        }
    }

    fn open(&self, _: &Request, inode: INodeNo, flags: OpenFlags, reply: ReplyOpen) {
        tracing::trace!(
            operation = "fuse.open",
            inode = u64::from(inode),
            ?flags,
            "FUSE callback invoked",
        );

        match self.open_file_handle(inode, flags) {
            Ok(file_handle) => {
                tracing::trace!(
                    operation = "fuse.open",
                    inode = u64::from(inode),
                    ?flags,
                    file_handle = u64::from(file_handle),
                    "FUSE callback succeeded",
                );

                reply.opened(file_handle, FopenFlags::FOPEN_DIRECT_IO);
            }
            Err(err) => {
                let errno = err.errno();
                log_fs_error!(
                    &err,
                    operation = "fuse.open",
                    inode = u64::from(inode),
                    ?flags,
                    ?errno,
                );

                reply.error(errno);
            }
        }
    }

    fn read(
        &self,
        _: &Request,
        inode: INodeNo,
        file_handle: FileHandle,
        offset: u64,
        size: u32,
        _: OpenFlags,
        _: Option<LockOwner>,
        reply: ReplyData,
    ) {
        tracing::trace!(
            operation = "fuse.read",
            inode = u64::from(inode),
            file_handle = u64::from(file_handle),
            offset,
            size,
            "FUSE callback invoked",
        );

        match self.file_snapshot(inode, file_handle) {
            Ok(file) => {
                let data = read_window(file.data(), offset, size);

                tracing::trace!(
                    operation = "fuse.read",
                    inode = u64::from(inode),
                    file_handle = u64::from(file_handle),
                    offset,
                    size,
                    bytes_read = data.len(),
                    "FUSE callback succeeded",
                );

                reply.data(data);
            }
            Err(err) => {
                let errno = err.errno();
                log_fs_error!(
                    &err,
                    operation = "fuse.read",
                    inode = u64::from(inode),
                    file_handle = u64::from(file_handle),
                    offset,
                    size,
                    ?errno,
                );

                reply.error(errno);
            }
        }
    }

    fn release(
        &self,
        _: &Request,
        inode: INodeNo,
        file_handle: FileHandle,
        _: OpenFlags,
        _: Option<LockOwner>,
        _: bool,
        reply: ReplyEmpty,
    ) {
        tracing::trace!(
            operation = "fuse.release",
            inode = u64::from(inode),
            file_handle = u64::from(file_handle),
            "FUSE callback invoked",
        );

        match self.release_file_handle(inode, file_handle) {
            Ok(()) => {
                tracing::trace!(
                    operation = "fuse.release",
                    inode = u64::from(inode),
                    file_handle = u64::from(file_handle),
                    "FUSE callback succeeded",
                );

                reply.ok();
            }
            Err(err) => {
                let errno = err.errno();
                log_fs_error!(
                    &err,
                    operation = "fuse.release",
                    inode = u64::from(inode),
                    file_handle = u64::from(file_handle),
                    ?errno,
                );

                reply.error(errno);
            }
        }
    }
}

impl KubeFs {
    pub(crate) fn new(reader: Box<dyn ClusterReader>, runtime_handle: Handle) -> Self {
        // SAFETY: getuid and getgid have no preconditions and only read the
        // calling process's credentials.
        let (owner_uid, owner_gid) = unsafe { (libc::getuid(), libc::getgid()) };

        Self {
            reader,
            registry: InodeRegistry::new(),
            open_files: OpenFileTable::new(),
            open_directories: OpenDirectoryTable::new(),
            runtime_handle,
            owner_uid,
            owner_gid,
            mount_time: SystemTime::now(),
        }
    }

    fn lookup_attr(&self, parent_inode: INodeNo, name: &OsStr) -> Result<FileAttr, FsError> {
        let parent_node = self
            .registry
            .node_for_inode(parent_inode)?
            .ok_or(FsError::NotFound)?;

        let child_name = name.to_str().ok_or(FsError::InvalidName)?;
        let child_node = self
            .runtime_handle
            .block_on(self.reader.child(&parent_node, child_name))?;
        let child_inode = self.registry.get_or_create_inode(child_node.clone())?;

        self.file_attr(child_inode, &child_node)
    }

    fn getattr_attr(&self, inode: INodeNo) -> Result<FileAttr, FsError> {
        let node = self
            .registry
            .node_for_inode(inode)?
            .ok_or(FsError::NotFound)?;

        self.file_attr(inode, &node)
    }

    fn file_attr(&self, inode: INodeNo, node: &Node) -> Result<FileAttr, FsError> {
        let size = self.file_size(node)?;
        let kind = node_file_type(node);
        let (permissions, link_count) = match node {
            Node::Object(_) => (0o444, 1),
            _ => (0o555, 2),
        };

        Ok(FileAttr {
            ino: inode,
            size,
            blocks: 0,
            atime: self.mount_time,
            mtime: self.mount_time,
            ctime: self.mount_time,
            crtime: self.mount_time,
            kind,
            perm: permissions,
            nlink: link_count,
            uid: self.owner_uid,
            gid: self.owner_gid,
            rdev: 0,
            blksize: BLOCK_SIZE,
            flags: 0,
        })
    }

    fn file_size(&self, node: &Node) -> Result<u64, FsError> {
        match node {
            Node::Object(object_id) => {
                let object_data = self
                    .runtime_handle
                    .block_on(self.reader.object_data(object_id))?;

                u64::try_from(object_data.len()).map_err(|_| FsError::FileSizeOverflow)
            }
            _ => Ok(0),
        }
    }

    fn open_directory_handle(
        &self,
        inode: INodeNo,
        flags: OpenFlags,
    ) -> Result<FileHandle, FsError> {
        let OpenAccMode::O_RDONLY = flags.acc_mode() else {
            return Err(FsError::WriteAccessRequested);
        };

        let entries = self.directory_entries(inode)?;
        let directory_handle = self.open_directories.insert(inode, entries)?;

        Ok(directory_handle)
    }

    fn directory_entries(&self, inode: INodeNo) -> Result<Vec<DirectoryEntry>, FsError> {
        let directory_node = self
            .registry
            .node_for_inode(inode)?
            .ok_or(FsError::NotFound)?;

        let parent_node = directory_node.parent().unwrap_or(Node::Root);
        let parent_inode = self
            .registry
            .inode_for_node(&parent_node)?
            .ok_or(FsError::ParentNotRegistered)?;

        let mut entries = vec![
            DirectoryEntry::new(inode, DOT_COOKIE, FileType::Directory, ".".into()),
            DirectoryEntry::new(
                parent_inode,
                DOT_DOT_COOKIE,
                FileType::Directory,
                "..".into(),
            ),
        ];

        let children = self
            .runtime_handle
            .block_on(self.reader.children(&directory_node))?;

        for (child_index, child) in children.into_iter().enumerate() {
            let child_kind = node_file_type(&child);
            let child_name: OsString = child.name().into();
            let child_inode = self.registry.get_or_create_inode(child)?;
            let index = u64::try_from(child_index).map_err(|_| FsError::DirectoryCookieOverflow)?;
            let next_offset = index
                .checked_add(FIRST_CHILD_COOKIE)
                .ok_or(FsError::DirectoryCookieOverflow)?;

            entries.push(DirectoryEntry::new(
                child_inode,
                next_offset,
                child_kind,
                child_name,
            ));
        }

        Ok(entries)
    }

    fn directory_snapshot(
        &self,
        inode: INodeNo,
        directory_handle: FileHandle,
    ) -> Result<Arc<OpenDirectory>, FsError> {
        let directory = self
            .open_directories
            .get(directory_handle)?
            .ok_or(FsError::DirectoryHandleNotFound)?;

        ensure_directory_matches_inode(&directory, inode)?;

        Ok(directory)
    }

    fn release_directory_handle(
        &self,
        inode: INodeNo,
        directory_handle: FileHandle,
    ) -> Result<(), FsError> {
        let directory = self
            .open_directories
            .remove(directory_handle)?
            .ok_or(FsError::DirectoryHandleNotFound)?;

        ensure_directory_matches_inode(&directory, inode)?;

        Ok(())
    }

    fn open_file_handle(&self, inode: INodeNo, flags: OpenFlags) -> Result<FileHandle, FsError> {
        let OpenAccMode::O_RDONLY = flags.acc_mode() else {
            return Err(FsError::WriteAccessRequested);
        };

        let object_id = match self.registry.node_for_inode(inode)? {
            Some(Node::Object(object_id)) => object_id,
            Some(_) => return Err(FsError::IsDirectory),
            None => return Err(FsError::NotFound),
        };

        let object_data = self
            .runtime_handle
            .block_on(self.reader.object_data(&object_id))?;
        let file_handle = self.open_files.insert(inode, object_data)?;

        Ok(file_handle)
    }

    fn file_snapshot(
        &self,
        inode: INodeNo,
        file_handle: FileHandle,
    ) -> Result<Arc<OpenFile>, FsError> {
        let file = self
            .open_files
            .get(file_handle)?
            .ok_or(FsError::FileHandleNotFound)?;

        ensure_file_matches_inode(&file, inode)?;

        Ok(file)
    }

    fn release_file_handle(&self, inode: INodeNo, file_handle: FileHandle) -> Result<(), FsError> {
        let file = self
            .open_files
            .remove(file_handle)?
            .ok_or(FsError::FileHandleNotFound)?;

        ensure_file_matches_inode(&file, inode)?;

        Ok(())
    }
}

fn read_window(data: &[u8], offset: u64, size: u32) -> &[u8] {
    let Ok(start) = usize::try_from(offset) else {
        return &[];
    };
    let Some(remaining) = data.get(start..) else {
        return &[];
    };
    let requested = usize::try_from(size).unwrap_or(usize::MAX);
    &remaining[..remaining.len().min(requested)]
}

fn node_file_type(node: &Node) -> FileType {
    match node {
        Node::Object(_) => FileType::RegularFile,
        _ => FileType::Directory,
    }
}

fn ensure_file_matches_inode(file: &OpenFile, requested_inode: INodeNo) -> Result<(), FsError> {
    let stored_inode = file.inode();

    if stored_inode != requested_inode {
        return Err(FsError::FileHandleInodeMismatch {
            requested_inode,
            stored_inode,
        });
    }
    Ok(())
}

fn ensure_directory_matches_inode(
    directory: &OpenDirectory,
    requested_inode: INodeNo,
) -> Result<(), FsError> {
    let stored_inode = directory.inode();

    if stored_inode != requested_inode {
        return Err(FsError::DirectoryHandleInodeMismatch {
            requested_inode,
            stored_inode,
        });
    }
    Ok(())
}
