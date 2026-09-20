use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    sync::{Arc, Mutex, PoisonError},
};

use fuser::{FileHandle, FileType, INodeNo};
use thiserror::Error;

const FIRST_DIRECTORY_HANDLE: u64 = 1;

#[derive(Debug, Error)]
pub(super) enum OpenDirectoryTableError {
    #[error("open-directory table lock poisoned")]
    LockPoisoned,

    #[error("directory-handle allocation exhausted")]
    HandleExhausted,
}

impl<T> From<PoisonError<T>> for OpenDirectoryTableError {
    fn from(_: PoisonError<T>) -> Self {
        OpenDirectoryTableError::LockPoisoned
    }
}

pub(super) struct OpenDirectoryTable {
    state: Mutex<State>,
}

pub(super) struct OpenDirectory {
    inode: INodeNo,
    entries: Vec<DirectoryEntry>,
}

impl OpenDirectory {
    pub(super) fn inode(&self) -> INodeNo {
        self.inode
    }

    pub(super) fn entries(&self) -> &[DirectoryEntry] {
        &self.entries
    }
}

pub(super) struct DirectoryEntry {
    inode: INodeNo,
    next_offset: u64,
    kind: FileType,
    name: OsString,
}

impl DirectoryEntry {
    pub(super) fn new(inode: INodeNo, next_offset: u64, kind: FileType, name: OsString) -> Self {
        Self {
            inode,
            next_offset,
            kind,
            name,
        }
    }

    pub(super) fn inode(&self) -> INodeNo {
        self.inode
    }

    pub(super) fn next_offset(&self) -> u64 {
        self.next_offset
    }

    pub(super) fn kind(&self) -> FileType {
        self.kind
    }

    pub(super) fn name(&self) -> &OsStr {
        &self.name
    }
}

struct State {
    entries: HashMap<FileHandle, Arc<OpenDirectory>>,
    next_handle: u64,
}

impl State {
    fn allocate_handle(&mut self) -> Result<FileHandle, OpenDirectoryTableError> {
        let handle = FileHandle(self.next_handle);
        self.next_handle = self
            .next_handle
            .checked_add(1)
            .ok_or(OpenDirectoryTableError::HandleExhausted)?;
        Ok(handle)
    }
}

impl OpenDirectoryTable {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(State {
                entries: HashMap::new(),
                next_handle: FIRST_DIRECTORY_HANDLE,
            }),
        }
    }

    pub(super) fn insert(
        &self,
        inode: INodeNo,
        entries: Vec<DirectoryEntry>,
    ) -> Result<FileHandle, OpenDirectoryTableError> {
        let mut state = self.state.lock()?;
        let directory_handle = state.allocate_handle()?;
        let open_directory = Arc::new(OpenDirectory { inode, entries });
        state.entries.insert(directory_handle, open_directory);
        Ok(directory_handle)
    }

    pub(super) fn get(
        &self,
        directory_handle: FileHandle,
    ) -> Result<Option<Arc<OpenDirectory>>, OpenDirectoryTableError> {
        let state = self.state.lock()?;
        Ok(state.entries.get(&directory_handle).cloned())
    }

    pub(super) fn remove(
        &self,
        directory_handle: FileHandle,
    ) -> Result<Option<Arc<OpenDirectory>>, OpenDirectoryTableError> {
        let mut state = self.state.lock()?;
        Ok(state.entries.remove(&directory_handle))
    }
}
