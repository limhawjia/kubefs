use std::{
    collections::HashMap,
    sync::{Arc, Mutex, PoisonError},
};

use fuser::{FileHandle, INodeNo};
use thiserror::Error;

const FIRST_FILE_HANDLE: u64 = 1;

#[derive(Debug, Error)]
pub(super) enum OpenFileTableError {
    #[error("open-file table lock poisoned")]
    LockPoisoned,

    #[error("file-handle allocation exhausted")]
    HandleExhausted,
}

impl<T> From<PoisonError<T>> for OpenFileTableError {
    fn from(_: PoisonError<T>) -> Self {
        OpenFileTableError::LockPoisoned
    }
}

pub(super) struct OpenFileTable {
    state: Mutex<State>,
}

pub(super) struct OpenFile {
    inode: INodeNo,
    data: Vec<u8>,
}

impl OpenFile {
    pub(super) fn inode(&self) -> INodeNo {
        self.inode
    }

    pub(super) fn data(&self) -> &[u8] {
        &self.data
    }
}

struct State {
    entries: HashMap<FileHandle, Arc<OpenFile>>,
    next_handle: u64,
}

impl State {
    fn allocate_handle(&mut self) -> Result<FileHandle, OpenFileTableError> {
        let handle = FileHandle(self.next_handle);
        self.next_handle = self
            .next_handle
            .checked_add(1)
            .ok_or(OpenFileTableError::HandleExhausted)?;
        Ok(handle)
    }
}

impl OpenFileTable {
    pub(super) fn new() -> Self {
        Self {
            state: Mutex::new(State {
                entries: HashMap::new(),
                next_handle: FIRST_FILE_HANDLE,
            }),
        }
    }

    pub(super) fn insert(
        &self,
        inode: INodeNo,
        data: Vec<u8>,
    ) -> Result<FileHandle, OpenFileTableError> {
        let mut state = self.state.lock()?;
        let file_handle = state.allocate_handle()?;
        let open_file = Arc::new(OpenFile { inode, data });
        state.entries.insert(file_handle, open_file);
        Ok(file_handle)
    }

    pub(super) fn get(
        &self,
        file_handle: FileHandle,
    ) -> Result<Option<Arc<OpenFile>>, OpenFileTableError> {
        let state = self.state.lock()?;
        Ok(state.entries.get(&file_handle).cloned())
    }

    pub(super) fn remove(
        &self,
        file_handle: FileHandle,
    ) -> Result<Option<Arc<OpenFile>>, OpenFileTableError> {
        let mut state = self.state.lock()?;
        Ok(state.entries.remove(&file_handle))
    }
}
